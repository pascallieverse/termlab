//! `Termlab`: the session layer's public API.
//!
//! Runs inside `termlabd` (the per-user daemon that owns the ptys); the desktop app reaches
//! it through `ipc`. Nothing here depends on UI code.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::runtime::Handle;
use tokio::sync::broadcast;

use crate::adapter::{
    AgentAdapter, AgentInfo, CommandSpec, HookSetup, Installed, LaunchRequest, PastSession,
    PlanReport, RunningSession, all_adapters, claude_project_dir, claude_settings_json,
    codex_rollout_path, one_line,
};
use crate::db::{Db, HookEventRow};
use crate::env::UserEnv;
use crate::error::{CoreError, Result};
use crate::git::{self, ChangedFile};
use crate::hooks::{self, HookMessage, MAX_MESSAGE_BYTES};
use crate::model::{
    AgentId, Approval, ApprovalChoice, Billing, Folder, FolderId, Isolation, Millis,
    PermissionPreset, PlanUsage, SESSION_COLORS, SessionId, SessionOrigin, SessionRecord,
    SessionStatus, SessionView, SubAgent, TermSize, Usage, now_ms,
};
use crate::pty::{Attachment, ExitInfo, PtyProcess, SpawnSpec};
use crate::scrollback::{self, DEFAULT_SCROLLBACK_BYTES, Scrollback};
use crate::status::{self, answer_keys, heuristic_status, parse_prompt};
use crate::transcript::{self, Progress, TimelineEntry, TimelineKind};

const MONITOR_INTERVAL: Duration = Duration::from_millis(500);
const USAGE_INTERVAL_MS: Millis = 5_000;
const ID_CHECK_INTERVAL_MS: Millis = 3_000;
/// A finished sub-agent stays listed this long.
const SUBAGENT_RECENT_MS: Millis = 10 * 60 * 1000;
/// At most this many sub-agents are listed per session.
const MAX_SUBAGENTS: usize = 12;
/// Screen changes this soon after a keystroke are the agent echoing your typing.
const TYPING_ECHO_MS: Millis = 1_000;
const PREVIEW_LINES: usize = 4;
/// A hook's "Needs you" yields to the screen once the prompt has been gone this long.
const STALE_NEEDS_YOU_MS: Millis = 3_000;
/// Output this long after a hook's "Idle" means the agent is active again.
const IDLE_HOOK_GRACE_MS: Millis = 1_500;
/// Delay between pasting a message and pressing Enter, so TUIs see them separately.
const SUBMIT_DELAY: Duration = Duration::from_millis(60);
/// Live scrollback is also saved this often, in case the daemon dies.
const SCROLLBACK_SAVE_INTERVAL_MS: Millis = 30_000;

/// Where Termlab keeps its files.
#[derive(Debug, Clone)]
pub struct DataDirs {
    pub root: PathBuf,
}

impl DataDirs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The OS app-data dir (`~/Library/Application Support/dev.termlab.Termlab` on macOS).
    pub fn for_user() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("dev", "termlab", "Termlab")
            .ok_or(CoreError::NoDataDir)?;
        Ok(Self::new(dirs.data_dir()))
    }

    pub fn db_path(&self) -> PathBuf {
        self.root.join("termlab.sqlite")
    }

    pub fn scrollback_path(&self, id: SessionId) -> PathBuf {
        self.root.join("scrollback").join(format!("{id}.log"))
    }

    /// Where `termlab-hook` delivers events.
    pub fn hook_socket(&self) -> PathBuf {
        self.root.join("hook.sock")
    }

    /// Where the app talks to `termlabd`.
    pub fn daemon_socket(&self) -> PathBuf {
        self.root.join("termlabd.sock")
    }

    pub fn claude_settings_path(&self) -> PathBuf {
        self.root.join("hooks").join("claude-settings.json")
    }

    /// The last plan limits reported, kept across restarts.
    pub fn plan_usage_path(&self) -> PathBuf {
        self.root.join("plan-usage.json")
    }
}

pub struct TermlabConfig {
    pub dirs: DataDirs,
    pub env: UserEnv,
    pub adapters: Vec<Box<dyn AgentAdapter>>,
    pub scrollback_cap: usize,
    /// The `termlab-hook` binary; `None` disables hooks (screen heuristics only).
    pub hook_binary: Option<PathBuf>,
}

impl TermlabConfig {
    pub fn new(dirs: DataDirs, env: UserEnv) -> Self {
        Self {
            dirs,
            env,
            adapters: all_adapters(),
            scrollback_cap: DEFAULT_SCROLLBACK_BYTES,
            hook_binary: None,
        }
    }
}

/// Things the UI should react to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CoreEvent {
    SessionUpdated { session: Box<SessionView> },
    SessionRemoved { id: SessionId },
    FoldersChanged,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StartMode {
    #[default]
    New,
    /// Continue an agent session recorded on disk.
    #[serde(rename_all = "camelCase")]
    Resume { agent_session_id: String },
    /// A new session branching from an agent session.
    #[serde(rename_all = "camelCase")]
    Fork { agent_session_id: String },
    /// Attach to a background session the agent runs outside Termlab.
    Attach { running: RunningSession },
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub folder_id: FolderId,
    pub agent: AgentId,
    #[serde(default)]
    pub model: Option<String>,
    pub permission: PermissionPreset,
    #[serde(default)]
    pub first_message: Option<String>,
    #[serde(default)]
    pub size: Option<TermSize>,
    #[serde(default = "default_true")]
    pub notify: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub mode: StartMode,
}

/// A session the agents recorded on disk, for the Resume tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    #[serde(flatten)]
    pub past: PastSession,
    /// The project folder it belongs to, when that folder has been added.
    pub folder_id: Option<FolderId>,
    /// The Termlab session already tracking this agent session, if any.
    pub termlab_session: Option<SessionId>,
    /// The agent is running this session right now, outside Termlab.
    pub running_elsewhere: bool,
}

/// An agent session running outside Termlab, for the Resume tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct External {
    #[serde(flatten)]
    pub running: RunningSession,
    pub folder_id: Option<FolderId>,
    /// Can be attached to (otherwise it's open in another terminal window).
    pub attachable: bool,
    /// The Termlab session already attached to it, if live.
    pub termlab_session: Option<SessionId>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub imported: usize,
    pub already_present: usize,
    pub folders_added: usize,
    /// Sessions whose folder no longer exists.
    pub skipped: usize,
}

/// The inspector's data for one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetails {
    pub cwd: PathBuf,
    pub branch: Option<String>,
    pub files: Vec<ChangedFile>,
    pub usage: Option<Usage>,
    pub context_percent: Option<u8>,
    /// The command that resumes this session in a terminal.
    pub resume_command: Option<String>,
    pub can_fork: bool,
    pub can_resume: bool,
}

/// The user's per-launch choices.
struct LaunchChoice {
    model: Option<String>,
    permission: PermissionPreset,
    first_message: Option<String>,
}

/// Live, in-memory state of a running session.
#[derive(Default)]
struct LiveMeta {
    hook_status: Option<SessionStatus>,
    hook_at: Millis,
    approval_detail: Option<String>,
    // Last values sent to the UI, to only emit changes.
    status: Option<SessionStatus>,
    preview: Vec<String>,
    approval: Option<Approval>,
    usage: Option<Usage>,
    usage_checked_at: Millis,
    id_checked_at: Millis,
    saved_at: Millis,
    /// Hash of the visible text, and when it last changed.
    screen_hash: Option<u64>,
    changed_at: Millis,
    /// `last_output_at` when the scrollback was last saved.
    saved_output_at: Millis,
    /// How far the transcript has been searched for renames made in the agent.
    rename_scan: Option<(PathBuf, u64)>,
    /// The session's sub-agents: working now, or just finished.
    subagents: Vec<SubAgent>,
}

pub struct Termlab {
    db: Mutex<Db>,
    live: RwLock<HashMap<SessionId, Arc<PtyProcess>>>,
    meta: Mutex<HashMap<SessionId, LiveMeta>>,
    stop_requested: Mutex<HashSet<SessionId>>,
    /// Running sessions to forget once they exit ("Remove" on a live session).
    remove_on_exit: Mutex<HashSet<SessionId>>,
    /// Set by `shutdown`: sessions killed from then on keep the record shutdown wrote.
    shutting_down: std::sync::atomic::AtomicBool,
    installed: RwLock<Option<HashMap<AgentId, Installed>>>,
    /// The plan limits each agent last reported.
    plan_usage: Mutex<HashMap<AgentId, PlanUsage>>,
    /// When each agent was last asked for them.
    plan_asked_at: Mutex<HashMap<AgentId, Millis>>,
    config: TermlabConfig,
    events: broadcast::Sender<CoreEvent>,
    rt: Handle,
}

impl Termlab {
    /// Opens the database (running migrations) and returns the service. Must be called
    /// from inside a Tokio runtime. Call [`Termlab::start_background`] to run the status
    /// monitor and hook receiver.
    pub fn open(config: TermlabConfig) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&config.dirs.root)?;
        let db = Db::open(&config.dirs.db_path())?;
        let orphans = db.mark_orphans_ended(now_ms())?;
        if orphans > 0 {
            tracing::info!(orphans, "marked sessions from a previous run as ended");
        }
        if let Some(bin) = &config.hook_binary {
            let theirs = crate::adapter::claude_user_status_line(&config.env);
            let json = serde_json::to_vec_pretty(&claude_settings_json(bin, theirs.as_deref()))
                .map_err(|e| CoreError::InvalidInput(e.to_string()))?;
            scrollback::write_atomically(&config.dirs.claude_settings_path(), &json)?;
        }
        let (events, _) = broadcast::channel(1024);
        Ok(Arc::new(Self {
            db: Mutex::new(db),
            live: RwLock::new(HashMap::new()),
            meta: Mutex::new(HashMap::new()),
            stop_requested: Mutex::new(HashSet::new()),
            remove_on_exit: Mutex::new(HashSet::new()),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
            installed: RwLock::new(None),
            plan_usage: Mutex::new(load_plan_usage(&config.dirs.plan_usage_path())),
            plan_asked_at: Mutex::new(HashMap::new()),
            config,
            events,
            rt: Handle::current(),
        }))
    }

    /// Starts the status monitor and (if hooks are configured) the hook receiver.
    pub fn start_background(self: &Arc<Self>) -> Result<()> {
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("status-monitor".into())
            .spawn(move || monitor_loop(weak))?;
        if self.config.hook_binary.is_some() {
            self.serve_hooks(&self.config.dirs.hook_socket())?;
        }
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("plan-usage".into())
            .spawn(move || plan_usage_loop(weak))?;
        // Sessions that were running when the service last stopped (or crashed) continue
        // where they left off. Done off the startup path: resuming detects agents first.
        let this = Arc::clone(self);
        std::thread::Builder::new()
            .name("resume-interrupted".into())
            .spawn(move || this.resume_interrupted())?;
        Ok(())
    }

    /// Resumes, with the agent's resume command, every session marked as interrupted.
    /// Sessions whose agent session id isn't known can't be resumed and stay ended.
    pub fn resume_interrupted(self: &Arc<Self>) {
        let pending = match self.db().take_sessions_to_resume() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "could not read interrupted sessions");
                return;
            }
        };
        for rec in pending {
            if rec.agent_session_id.is_none() {
                tracing::info!(session = %rec.id, "interrupted session has no agent session id; not resuming");
                continue;
            }
            match self.restart(rec.id, rec.term_size) {
                Ok(_) => tracing::info!(session = %rec.id, "resumed interrupted session"),
                Err(e) => {
                    tracing::warn!(session = %rec.id, error = %e, "could not resume interrupted session")
                }
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.events.subscribe()
    }

    pub fn env(&self) -> &UserEnv {
        &self.config.env
    }

    pub fn dirs(&self) -> &DataDirs {
        &self.config.dirs
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Db> {
        self.db.lock().expect("db lock")
    }

    fn emit(&self, e: CoreEvent) {
        let _ = self.events.send(e);
    }

    fn emit_session(&self, id: SessionId) {
        if let Ok(view) = self.session(id) {
            self.emit(CoreEvent::SessionUpdated {
                session: Box::new(view),
            });
        }
    }

    // ---- folders ----------------------------------------------------------

    pub fn add_folder(&self, path: &Path) -> Result<Folder> {
        let path = path
            .canonicalize()
            .map_err(|_| CoreError::NotADirectory(path.to_path_buf()))?;
        if !path.is_dir() {
            return Err(CoreError::NotADirectory(path));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let folder = self.db().add_folder(&name, &path, now_ms())?;
        self.emit(CoreEvent::FoldersChanged);
        Ok(folder)
    }

    pub fn list_folders(&self) -> Result<Vec<Folder>> {
        self.db().list_folders()
    }

    pub fn move_folder(&self, id: FolderId, before: Option<FolderId>) -> Result<()> {
        self.db().move_folder(id, before)?;
        self.emit(CoreEvent::FoldersChanged);
        Ok(())
    }

    /// Removes a folder and its session history. Refuses while it has live sessions.
    pub fn remove_folder(&self, id: FolderId) -> Result<()> {
        let records = self.db().list_sessions()?;
        if records
            .iter()
            .any(|s| s.folder_id == id && self.is_live(s.id))
        {
            return Err(CoreError::InvalidInput(
                "stop this folder's running sessions first".into(),
            ));
        }
        self.db().remove_folder(id)?;
        for s in records.iter().filter(|s| s.folder_id == id) {
            let _ = std::fs::remove_file(self.config.dirs.scrollback_path(s.id));
        }
        self.emit(CoreEvent::FoldersChanged);
        Ok(())
    }

    // ---- agents -----------------------------------------------------------

    fn installed_map(&self) -> HashMap<AgentId, Installed> {
        if let Some(map) = self.installed.read().expect("installed lock").as_ref() {
            return map.clone();
        }
        self.refresh_agents();
        self.installed
            .read()
            .expect("installed lock")
            .clone()
            .unwrap_or_default()
    }

    /// Detects installed agents (spawns `--version` and login-status processes).
    pub fn refresh_agents(&self) {
        let map: HashMap<_, _> = self
            .config
            .adapters
            .iter()
            .filter_map(|a| a.detect(&self.config.env))
            .map(|i| (i.agent, i))
            .collect();
        *self.installed.write().expect("installed lock") = Some(map);
    }

    /// What each agent's service last said about the account: connection, plan and limits.
    /// Windows that have reset since are left out (their usage is no longer known).
    pub fn plan_usage(&self) -> Vec<PlanUsage> {
        let now = now_ms();
        let mut all: Vec<PlanUsage> = self
            .plan_usage
            .lock()
            .expect("plan usage lock")
            .values()
            .cloned()
            .collect();
        for usage in &mut all {
            usage
                .windows
                .retain(|w| w.resets_at.is_none_or(|t| t > now));
        }
        // Keep an agent with nothing to chart if there's something to say about it.
        all.retain(|u| !u.windows.is_empty() || u.connected.is_some() || u.note.is_some());
        all.sort_by_key(|u| u.agent.to_string());
        all
    }

    /// Stores what an agent's service said about the account. A `partial` report (Claude's
    /// status line: only the 5-hour and weekly windows) updates those windows and keeps
    /// everything else; a full one replaces the entry.
    fn record_plan_usage(&self, agent: AgentId, report: PlanReport, partial: bool) {
        // Nothing in this report (e.g. a status line before the session's first response).
        let says_nothing = report.connected.is_none() && report.note.is_none();
        if report.windows.is_empty() && (partial || says_nothing) {
            return;
        }
        let mut all = self.plan_usage.lock().expect("plan usage lock");
        let before = all.get(&agent).cloned();
        // An older measurement doesn't replace a newer one.
        if before.as_ref().is_some_and(|b| b.updated_at > report.at) {
            return;
        }
        let usage = match (&before, partial) {
            (Some(b), true) => {
                let mut merged = b.windows.clone();
                for w in report.windows {
                    match merged.iter_mut().find(|m| m.id == w.id) {
                        Some(m) => *m = w,
                        None => merged.push(w),
                    }
                }
                PlanUsage {
                    windows: merged,
                    updated_at: report.at,
                    ..b.clone()
                }
            }
            _ => PlanUsage {
                agent,
                windows: report.windows,
                updated_at: report.at,
                connected: report.connected,
                plan: report.plan,
                note: report.note,
            },
        };
        let changed = before.as_ref().is_none_or(|b| {
            b.windows != usage.windows
                || b.connected != usage.connected
                || b.plan != usage.plan
                || b.note != usage.note
        });
        all.insert(agent, usage);
        if changed {
            let list: Vec<&PlanUsage> = all.values().collect();
            if let Ok(json) = serde_json::to_vec(&list) {
                let _ = scrollback::write_atomically(&self.config.dirs.plan_usage_path(), &json);
            }
        }
    }

    /// Asks every agent that can report plan limits and is due. `forced`: the user asked, so
    /// agents that are normally only asked while a session of theirs runs are asked too.
    pub fn refresh_plan_usage(&self, forced: bool) {
        /// However it's triggered, an agent isn't asked more often than this.
        const MIN_GAP_MS: Millis = 20_000;
        let installed = self.installed_map();
        let live_agents: HashSet<AgentId> = self
            .db()
            .list_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| self.is_live(s.id))
            .map(|s| s.agent)
            .collect();
        for adapter in &self.config.adapters {
            let agent = adapter.id();
            let (Some(poll), Some(found)) = (adapter.plan_usage_poll(), installed.get(&agent))
            else {
                continue;
            };
            let now = now_ms();
            {
                let mut asked = self.plan_asked_at.lock().expect("plan asked lock");
                let since = now - asked.get(&agent).copied().unwrap_or(0);
                let due = if forced {
                    since >= MIN_GAP_MS
                } else {
                    since >= poll.every_ms
                        && (!poll.only_while_live || live_agents.contains(&agent))
                };
                if !due {
                    continue;
                }
                asked.insert(agent, now);
            }
            if let Some(report) =
                adapter.plan_usage(&found.path, &self.config.env, &self.config.dirs.root)
            {
                self.record_plan_usage(agent, report, false);
            }
        }
    }

    /// Every known agent with what it supports and whether it's installed.
    pub fn agents(&self) -> Vec<AgentInfo> {
        let installed = self.installed_map();
        self.config
            .adapters
            .iter()
            .map(|a| a.info(&self.config.env, installed.get(&a.id()).cloned()))
            .collect()
    }

    fn adapter(&self, agent: AgentId) -> Result<&dyn AgentAdapter> {
        self.config
            .adapters
            .iter()
            .find(|a| a.id() == agent)
            .map(|a| a.as_ref())
            .ok_or(CoreError::AgentNotInstalled(agent))
    }

    fn installed(&self, agent: AgentId) -> Result<Installed> {
        self.installed_map()
            .remove(&agent)
            .ok_or(CoreError::AgentNotInstalled(agent))
    }

    fn hook_setup(&self) -> Option<HookSetup> {
        self.config.hook_binary.as_ref().map(|b| HookSetup {
            binary: b.clone(),
            claude_settings: self.config.dirs.claude_settings_path(),
        })
    }

    fn launch_request(
        &self,
        agent: AgentId,
        folder: &Folder,
        choice: LaunchChoice,
        id: SessionId,
        hooks: bool,
    ) -> Result<LaunchRequest> {
        Ok(LaunchRequest {
            session_id: id,
            program: self.installed(agent)?.path,
            cwd: folder.path.clone(),
            model: choice.model.filter(|m| !m.trim().is_empty()),
            permission: choice.permission,
            first_message: choice.first_message,
            hooks: if hooks { self.hook_setup() } else { None },
        })
    }

    fn command_for(&self, req: &StartRequest, id: SessionId, hooks: bool) -> Result<CommandSpec> {
        let adapter = self.adapter(req.agent)?;
        let folder = self.db().folder(req.folder_id)?;
        let lr = self.launch_request(
            req.agent,
            &folder,
            LaunchChoice {
                model: req.model.clone(),
                permission: req.permission,
                first_message: req.first_message.clone(),
            },
            id,
            hooks,
        )?;
        let unsupported =
            || CoreError::InvalidInput(format!("{} can't do that", req.agent.display_name()));
        match &req.mode {
            StartMode::New => Ok(adapter.launch(&lr)),
            StartMode::Resume { agent_session_id } => adapter
                .resume(agent_session_id, &lr)
                .ok_or_else(unsupported),
            StartMode::Fork { agent_session_id } => {
                adapter.fork(agent_session_id, &lr).ok_or_else(unsupported)
            }
            StartMode::Attach { running } => adapter.attach(running, &lr).ok_or_else(|| {
                CoreError::InvalidInput(
                    "that session is open in another terminal window and can't be attached to"
                        .into(),
                )
            }),
        }
    }

    /// The exact command `start_session` would run, for the "Will run" preview.
    pub fn preview_command(&self, req: &StartRequest, id: SessionId) -> Result<String> {
        // Hook plumbing (`--settings`, `-c notify=…`) is included: it's part of what runs.
        Ok(self.command_for(req, id, true)?.display())
    }

    fn billing(&self, agent: AgentId) -> Billing {
        let adapter = match self.adapter(agent) {
            Ok(a) => a,
            Err(_) => return Billing::Unknown,
        };
        if crate::env::billing_for(&self.config.env, adapter.api_key_vars()) == Billing::ApiKey {
            return Billing::ApiKey;
        }
        self.installed(agent)
            .map(|i| i.login_billing)
            .unwrap_or(Billing::Unknown)
    }

    fn api_key_vars_for(&self, agent: AgentId) -> Vec<String> {
        self.adapter(agent)
            .map(|a| {
                a.api_key_vars()
                    .iter()
                    .filter(|k| self.config.env.get(k).is_some_and(|v| !v.trim().is_empty()))
                    .map(|k| k.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---- sessions ---------------------------------------------------------

    fn view(&self, record: SessionRecord) -> SessionView {
        let live = self.is_live(record.id);
        let meta = self.meta.lock().expect("meta lock");
        let m = meta.get(&record.id);
        let usage = m.and_then(|m| m.usage.clone());
        SessionView {
            api_key_vars: self.api_key_vars_for(record.agent),
            subagents: m
                .filter(|_| live)
                .map(|m| m.subagents.clone())
                .unwrap_or_default(),
            live,
            preview: m.map(|m| m.preview.clone()).unwrap_or_default(),
            approval: m.and_then(|m| m.approval.clone()),
            context_percent: usage.as_ref().and_then(Usage::context_percent),
            usage,
            record,
        }
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionView>> {
        let records = self.db().list_sessions()?;
        Ok(records.into_iter().map(|r| self.view(r)).collect())
    }

    pub fn session(&self, id: SessionId) -> Result<SessionView> {
        let r = self.db().session(id)?;
        Ok(self.view(r))
    }

    fn record(&self, id: SessionId) -> Result<SessionRecord> {
        self.db().session(id)
    }

    pub fn is_live(&self, id: SessionId) -> bool {
        self.live.read().expect("live lock").contains_key(&id)
    }

    fn live(&self, id: SessionId) -> Result<Arc<PtyProcess>> {
        self.live
            .read()
            .expect("live lock")
            .get(&id)
            .cloned()
            .ok_or(CoreError::SessionNotLive(id))
    }

    /// Starts a session. `id` lets the caller fix the id it previewed.
    pub fn start_session(
        self: &Arc<Self>,
        req: &StartRequest,
        id: Option<SessionId>,
    ) -> Result<SessionView> {
        let id = id.unwrap_or_default();
        if let StartMode::Resume { agent_session_id } = &req.mode {
            self.ensure_not_running_elsewhere(req.agent, agent_session_id)?;
        }
        let spec = self.command_for(req, id, true)?;
        let folder = self.db().folder(req.folder_id)?;
        let now = now_ms();
        let record = SessionRecord {
            id,
            folder_id: req.folder_id,
            agent: req.agent,
            title: req
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| default_title(req)),
            model: req.model.clone().filter(|m| !m.trim().is_empty()),
            permission_mode: Some(req.permission),
            isolation: Isolation::Checkout,
            agent_session_id: spec.agent_session_id.clone(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at: now,
            last_activity_at: now,
            billing: self.billing(req.agent),
            start_commit: git::head_commit(&self.config.env, &folder.path),
            transcript_path: None,
            notify: req.notify,
            origin: if matches!(req.mode, StartMode::Attach { .. }) {
                SessionOrigin::Attached
            } else {
                SessionOrigin::Termlab
            },
            term_size: Some(req.size.unwrap_or_default()),
            color: None,
        };
        self.db().insert_session(&record)?;
        let size = req.size.unwrap_or_default();
        match self.spawn(
            record.id,
            &spec,
            size,
            Scrollback::new(self.config.scrollback_cap),
        ) {
            Ok(()) => self.session(id),
            Err(e) => {
                let _ = self.db().delete_session(id);
                Err(e)
            }
        }
    }

    /// Continues a finished session in place (same Termlab session, earlier output kept),
    /// using the agent's resume command.
    pub fn restart(self: &Arc<Self>, id: SessionId, size: Option<TermSize>) -> Result<SessionView> {
        if self.is_live(id) {
            return Err(CoreError::InvalidInput(
                "the session is still running".into(),
            ));
        }
        let rec = self.record(id)?;
        let agent_session_id = rec.agent_session_id.clone().ok_or_else(|| {
            CoreError::InvalidInput(
                "this session's agent id isn't known, so it can't be resumed".into(),
            )
        })?;
        // An attached session is re-attached: the agent kept running it in the background.
        // Anything else is resumed, unless the agent is running it somewhere right now.
        let mode = if rec.origin == SessionOrigin::Attached {
            let ext = self
                .external_sessions()
                .into_iter()
                .find(|e| {
                    e.running.agent == rec.agent && e.running.agent_session_id == agent_session_id
                })
                .ok_or_else(|| {
                    CoreError::InvalidInput("that background session isn't running any more".into())
                })?;
            StartMode::Attach {
                running: ext.running,
            }
        } else {
            self.ensure_not_running_elsewhere(rec.agent, &agent_session_id)?;
            StartMode::Resume { agent_session_id }
        };
        let req = StartRequest {
            folder_id: rec.folder_id,
            agent: rec.agent,
            model: rec.model.clone(),
            permission: rec.permission_mode.unwrap_or(PermissionPreset::AskFirst),
            first_message: None,
            size,
            notify: rec.notify,
            title: None,
            mode,
        };
        let spec = self.command_for(&req, id, true)?;
        let history = scrollback::load(&self.config.dirs.scrollback_path(id))?.unwrap_or_default();
        let mut sb = Scrollback::with_contents(self.config.scrollback_cap, &history);
        sb.push(b"\r\n\x1b[2m[termlab] resumed\x1b[0m\r\n");
        self.db()
            .set_exited(id, SessionStatus::Running, None, now_ms())?;
        let size = size.or(rec.term_size).unwrap_or_default();
        self.db().set_term_size(id, size)?;
        self.spawn(id, &spec, size, sb)?;
        self.session(id)
    }

    fn spawn(
        self: &Arc<Self>,
        id: SessionId,
        spec: &CommandSpec,
        size: TermSize,
        scrollback: Scrollback,
    ) -> Result<()> {
        let mut additions = spec.env.clone();
        if self.config.hook_binary.is_some() {
            additions.push((
                "TERMLAB_HOOK_SOCKET".into(),
                self.config
                    .dirs
                    .hook_socket()
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        let env = self.config.env.for_spawn(&additions).into_iter().collect();
        let spawn = SpawnSpec {
            program: spec.program.clone(),
            args: spec.args.clone(),
            cwd: spec.cwd.clone(),
            env,
            size,
        };
        tracing::info!(session = %id, command = %spec.display(), "starting session");
        let pty = Arc::new(PtyProcess::spawn_with_history(&spawn, scrollback)?);
        self.meta
            .lock()
            .expect("meta lock")
            .insert(id, LiveMeta::default());
        self.live
            .write()
            .expect("live lock")
            .insert(id, pty.clone());
        self.emit_session(id);

        let this = Arc::clone(self);
        self.rt.spawn(async move {
            let info = pty.wait().await;
            this.on_exit(id, &pty, info);
        });
        Ok(())
    }

    fn on_exit(&self, id: SessionId, pty: &PtyProcess, info: ExitInfo) {
        if self.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        if self.remove_on_exit.lock().expect("remove lock").remove(&id) {
            self.stop_requested.lock().expect("stop lock").remove(&id);
            self.live.write().expect("live lock").remove(&id);
            if let Err(e) = self.forget(id) {
                tracing::warn!(session = %id, error = %e, "could not remove session");
            }
            return;
        }
        if let Err(e) = pty.flush_scrollback(&self.config.dirs.scrollback_path(id)) {
            tracing::warn!(session = %id, error = %e, "could not save scrollback");
        }
        let stopped = self.stop_requested.lock().expect("stop lock").remove(&id);
        let status = if info.success || stopped {
            SessionStatus::Done
        } else {
            SessionStatus::Error
        };
        tracing::info!(session = %id, code = info.code, ?status, "session exited");
        if let Err(e) = self.db().set_exited(id, status, Some(info.code), now_ms()) {
            tracing::warn!(session = %id, error = %e, "could not record exit");
        }
        // Keep the last usage figures for the finished session's view.
        if let Some(m) = self.meta.lock().expect("meta lock").get_mut(&id) {
            m.approval = None;
            m.hook_status = None;
        }
        self.live.write().expect("live lock").remove(&id);
        self.emit_session(id);
    }

    /// Everything printed so far plus a live event stream. For a finished session the
    /// snapshot comes from its saved scrollback file and the stream is already closed.
    pub fn attach(&self, id: SessionId) -> Result<Attachment> {
        if let Ok(pty) = self.live(id) {
            return Ok(pty.attach());
        }
        let record = self.record(id)?;
        let snapshot = scrollback::load(&self.config.dirs.scrollback_path(id))?.unwrap_or_default();
        let (_, events) = broadcast::channel(1);
        Ok(Attachment {
            snapshot,
            events,
            exited: Some(ExitInfo {
                code: record.exit_code.unwrap_or(-1),
                success: record.status == SessionStatus::Done,
            }),
            size: record.term_size,
        })
    }

    /// Raw keystrokes from the terminal view.
    pub fn write(&self, id: SessionId, data: impl Into<Vec<u8>>) -> Result<()> {
        self.live(id)?.write(data)
    }

    /// Types `text` into the agent's input and presses Enter. Multi-line text is sent as a
    /// bracketed paste when the agent enabled it, so newlines don't submit early.
    pub fn send_message(&self, id: SessionId, text: &str) -> Result<()> {
        let pty = self.live(id)?;
        let text = text.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            return pty.write(b"\r".to_vec());
        }
        let payload = if pty.screen().bracketed_paste {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text.replace('\n', "\r")
        };
        pty.write(payload.into_bytes())?;
        self.rt.spawn(async move {
            tokio::time::sleep(SUBMIT_DELAY).await;
            let _ = pty.write(b"\r".to_vec());
        });
        Ok(())
    }

    /// Sends `text` to every running session in a folder. Returns how many got it.
    pub fn broadcast(&self, folder_id: FolderId, text: &str) -> Result<usize> {
        let ids: Vec<SessionId> = self
            .db()
            .list_sessions()?
            .into_iter()
            .filter(|s| s.folder_id == folder_id)
            .map(|s| s.id)
            .filter(|id| self.is_live(*id))
            .collect();
        let mut sent = 0;
        for id in ids {
            if self.send_message(id, text).is_ok() {
                sent += 1;
            }
        }
        Ok(sent)
    }

    /// Answers the permission prompt on screen with the matching keystrokes.
    pub fn answer(&self, id: SessionId, choice: ApprovalChoice) -> Result<()> {
        let pty = self.live(id)?;
        let screen = pty.screen();
        let keys = answer_keys(&screen, choice, screen.application_cursor).ok_or_else(|| {
            CoreError::InvalidInput(
                "that option isn't on the agent's screen any more — answer in the terminal".into(),
            )
        })?;
        pty.write(keys)?;
        if let Some(m) = self.meta.lock().expect("meta lock").get_mut(&id) {
            m.hook_status = None;
            m.approval_detail = None;
        }
        Ok(())
    }

    pub fn resize(&self, id: SessionId, size: TermSize) -> Result<()> {
        self.live(id)?.resize(size)?;
        // Remembered so the saved output replays at the size it was drawn at.
        self.db().set_term_size(id, size)
    }

    /// Asks the agent to stop (SIGHUP, as when a terminal window closes).
    pub fn stop(&self, id: SessionId) -> Result<()> {
        let pty = self.live(id)?;
        self.stop_requested.lock().expect("stop lock").insert(id);
        pty.kill()
    }

    /// Stops the idle sessions in a folder. Returns how many were stopped.
    pub fn stop_idle(&self, folder_id: FolderId) -> Result<usize> {
        let ids: Vec<SessionId> = self
            .list_sessions()?
            .into_iter()
            .filter(|s| {
                s.record.folder_id == folder_id && s.live && s.record.status == SessionStatus::Idle
            })
            .map(|s| s.record.id)
            .collect();
        for id in &ids {
            let _ = self.stop(*id);
        }
        Ok(ids.len())
    }

    /// Forgets a finished session (its record and saved output).
    /// Removes a session from Termlab (its record and saved output). A running session is
    /// stopped first and removed once it has exited. The agent's own history is untouched.
    pub fn delete_session(&self, id: SessionId) -> Result<()> {
        if let Ok(pty) = self.live(id) {
            self.remove_on_exit.lock().expect("remove lock").insert(id);
            self.stop_requested.lock().expect("stop lock").insert(id);
            return pty.kill();
        }
        self.forget(id)
    }

    fn forget(&self, id: SessionId) -> Result<()> {
        self.db().delete_session(id)?;
        self.meta.lock().expect("meta lock").remove(&id);
        let _ = std::fs::remove_file(self.config.dirs.scrollback_path(id));
        self.emit(CoreEvent::SessionRemoved { id });
        Ok(())
    }

    pub fn set_title(&self, id: SessionId, title: &str) -> Result<()> {
        self.db().set_title(id, title.trim())?;
        self.emit_session(id);
        Ok(())
    }

    /// A new session of the same agent, branching from this one's conversation.
    pub fn fork(self: &Arc<Self>, id: SessionId, size: Option<TermSize>) -> Result<SessionView> {
        let rec = self.record(id)?;
        let agent_session_id = rec.agent_session_id.clone().ok_or_else(|| {
            CoreError::InvalidInput("this session's agent id isn't known yet".into())
        })?;
        if !self.adapter(rec.agent)?.can_fork() {
            return Err(CoreError::InvalidInput(format!(
                "{} can't fork sessions",
                rec.agent.display_name()
            )));
        }
        self.start_session(
            &StartRequest {
                folder_id: rec.folder_id,
                agent: rec.agent,
                model: rec.model.clone(),
                permission: rec.permission_mode.unwrap_or(PermissionPreset::AskFirst),
                first_message: None,
                size,
                notify: rec.notify,
                title: Some(format!("Fork: {}", rec.title)),
                mode: StartMode::Fork { agent_session_id },
            },
            None,
        )
    }

    /// The message that briefs another agent on this session: goal, progress, changes.
    pub fn handoff_message(&self, id: SessionId) -> Result<String> {
        let rec = self.record(id)?;
        let folder = self.db().folder(rec.folder_id)?;
        let progress = self
            .transcript_path(&rec, &folder.path)
            .map(|p| match rec.agent {
                AgentId::Claude => transcript::claude_progress(&p),
                AgentId::Codex => transcript::codex_progress(&p),
                _ => Progress::default(),
            })
            .unwrap_or_default();
        let base = rec
            .start_commit
            .clone()
            .or_else(|| git::head_commit(&self.config.env, &folder.path));
        let files = base
            .as_deref()
            .map(|b| git::changed_files(&self.config.env, &folder.path, b))
            .unwrap_or_default();

        let mut msg = format!(
            "You're taking over a task that {} was working on in this folder.\n\nGoal: {}\n",
            rec.agent.display_name(),
            progress
                .goal
                .as_deref()
                .map(|g| one_line(g, 1_500))
                .unwrap_or_else(|| rec.title.clone())
        );
        if let Some(last) = &progress.last_message {
            msg.push_str(&format!("\nWhere it got to: {}\n", one_line(last, 1_200)));
        }
        if !files.is_empty() {
            msg.push_str("\nFiles changed so far:\n");
            for f in files.iter().take(40) {
                let stats = match (f.added, f.deleted) {
                    (Some(a), Some(d)) => format!(" (+{a} -{d})"),
                    _ => " (binary)".into(),
                };
                let tag = if f.untracked { ", new" } else { "" };
                msg.push_str(&format!("- {}{stats}{tag}\n", f.path));
            }
            if files.len() > 40 {
                msg.push_str(&format!("- …and {} more\n", files.len() - 40));
            }
        }
        match &base {
            Some(b) => msg.push_str(&format!(
                "\nReview the changes so far with `git diff {}` before continuing, then finish the goal.",
                &b[..b.len().min(12)]
            )),
            None => msg.push_str("\nReview the current state of the folder, then finish the goal."),
        }
        Ok(msg)
    }

    /// Starts `agent` in the same folder, briefed on this session.
    pub fn hand_off(
        self: &Arc<Self>,
        id: SessionId,
        agent: AgentId,
        permission: PermissionPreset,
        model: Option<String>,
        size: Option<TermSize>,
    ) -> Result<SessionView> {
        let rec = self.record(id)?;
        let message = self.handoff_message(id)?;
        self.start_session(
            &StartRequest {
                folder_id: rec.folder_id,
                agent,
                model,
                permission,
                first_message: Some(message),
                size,
                notify: rec.notify,
                title: Some(format!("Hand-off: {}", rec.title)),
                mode: StartMode::New,
            },
            None,
        )
    }

    // ---- inspector, diff, timeline, discovery -------------------------------

    fn transcript_path(&self, rec: &SessionRecord, cwd: &Path) -> Option<PathBuf> {
        if let Some(p) = rec
            .transcript_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.exists())
        {
            return Some(p);
        }
        let id = rec.agent_session_id.as_deref()?;
        match rec.agent {
            AgentId::Claude => {
                let p = claude_project_dir(cwd, &self.config.env)?.join(format!("{id}.jsonl"));
                p.exists().then_some(p)
            }
            AgentId::Codex => codex_rollout_path(&self.config.env, id),
            _ => None,
        }
    }

    /// The session's sub-agents worth showing: those working now and those that finished in
    /// the last few minutes, working ones first, in the order they started.
    fn read_subagents(
        &self,
        rec: &SessionRecord,
        cwd: Option<&Path>,
        now: Millis,
    ) -> Vec<SubAgent> {
        let Some(asid) = rec.agent_session_id.as_deref() else {
            return Vec::new();
        };
        let Ok(adapter) = self.adapter(rec.agent) else {
            return Vec::new();
        };
        let transcript = cwd.and_then(|c| self.transcript_path(rec, c));
        let mut list = adapter.subagents(
            asid,
            transcript.as_deref(),
            rec.created_at,
            now,
            &self.config.env,
        );
        list.retain(|a| a.running || now - a.last_activity_at < SUBAGENT_RECENT_MS);
        list.sort_by(|a, b| {
            b.running
                .cmp(&a.running)
                .then(a.started_at.cmp(&b.started_at))
                .then_with(|| a.id.cmp(&b.id))
        });
        list.truncate(MAX_SUBAGENTS);
        list
    }

    /// Follows the transcript for what the user set inside the agent (Claude's /rename and
    /// /color) and applies what's new. Returns whether the session changed.
    fn apply_agent_marks(&self, id: SessionId, rec: &SessionRecord, cwd: &Path) -> bool {
        let Some(path) = self.transcript_path(rec, cwd) else {
            return false;
        };
        let Ok(adapter) = self.adapter(rec.agent) else {
            return false;
        };
        let from = {
            let meta = self.meta.lock().expect("meta lock");
            match meta.get(&id).and_then(|m| m.rename_scan.as_ref()) {
                Some((p, offset)) if *p == path => *offset,
                _ => 0,
            }
        };
        let (offset, marks) = adapter.transcript_marks(&path, from);
        self.meta
            .lock()
            .expect("meta lock")
            .entry(id)
            .or_default()
            .rename_scan = Some((path, offset));
        let db = self.db();
        let mut changed = false;

        // A colour Termlab knows, or "default" to clear it; anything else is ignored.
        if let Some(color) = marks.color.map(|c| c.to_lowercase()) {
            let color = if color == "default" {
                Some(None)
            } else {
                SESSION_COLORS
                    .contains(&color.as_str())
                    .then_some(Some(color))
            };
            if let Some(color) = color
                && color != rec.color
            {
                changed |= db.set_color(id, color.as_deref()).is_ok();
            }
        }

        // A new name is applied once, so a later rename in Termlab isn't overwritten by it.
        if let Some(title) = marks
            .title
            .map(|t| one_line(&t, 80))
            .filter(|t| !t.is_empty())
            && db.agent_title(id).ok().flatten().as_deref() != Some(title.as_str())
        {
            let _ = db.set_agent_title(id, &title);
            changed |= rec.title != title && db.set_title(id, &title).is_ok();
        }
        changed
    }

    /// The conversation's title from the agent's own transcript, when it has one.
    fn history_title(&self, rec: &SessionRecord, cwd: &Path) -> Option<String> {
        let asid = rec.agent_session_id.as_deref()?;
        let path = self.transcript_path(rec, cwd);
        let title =
            self.adapter(rec.agent)
                .ok()?
                .history_title(asid, path.as_deref(), &self.config.env)?;
        let title = one_line(&title, 80);
        (!title.is_empty()).then_some(title)
    }

    fn read_usage(&self, rec: &SessionRecord, cwd: &Path) -> Option<Usage> {
        let path = self.transcript_path(rec, cwd)?;
        match rec.agent {
            AgentId::Claude => transcript::claude_usage(&path, rec.billing == Billing::ApiKey),
            AgentId::Codex => transcript::codex_usage(&path),
            _ => None,
        }
    }

    pub fn details(&self, id: SessionId) -> Result<SessionDetails> {
        let rec = self.record(id)?;
        let folder = self.db().folder(rec.folder_id)?;
        let env = &self.config.env;
        let files = rec
            .start_commit
            .as_deref()
            .map(|b| git::changed_files(env, &folder.path, b))
            .unwrap_or_default();
        let usage = self.read_usage(&rec, &folder.path);
        let adapter = self.adapter(rec.agent)?;
        let resume_command = rec.agent_session_id.as_deref().and_then(|asid| {
            let lr = self
                .launch_request(
                    rec.agent,
                    &folder,
                    LaunchChoice {
                        model: rec.model.clone(),
                        permission: rec.permission_mode.unwrap_or(PermissionPreset::AskFirst),
                        first_message: None,
                    },
                    id,
                    false,
                )
                .ok()?;
            adapter.resume(asid, &lr).map(|c| c.display())
        });
        Ok(SessionDetails {
            branch: git::branch(env, &folder.path),
            files,
            context_percent: usage.as_ref().and_then(Usage::context_percent),
            usage,
            resume_command,
            can_fork: adapter.can_fork() && rec.agent_session_id.is_some(),
            can_resume: adapter.can_resume() && rec.agent_session_id.is_some(),
            cwd: folder.path,
        })
    }

    /// Unified diff since the session's start commit.
    pub fn diff(&self, id: SessionId) -> Result<String> {
        let rec = self.record(id)?;
        let folder = self.db().folder(rec.folder_id)?;
        let base = rec.start_commit.ok_or_else(|| {
            CoreError::InvalidInput(
                "this folder wasn't a git repository when the session started".into(),
            )
        })?;
        Ok(git::diff(&self.config.env, &folder.path, &base))
    }

    /// Hook events plus tool calls and messages from the transcript, oldest first.
    /// What one of a session's sub-agents did, from the agent's own record of it.
    pub fn subagent_timeline(&self, id: SessionId, subagent: &str) -> Result<Vec<TimelineEntry>> {
        let rec = self.record(id)?;
        let folder = self.db().folder(rec.folder_id)?;
        let Some(asid) = rec.agent_session_id.as_deref() else {
            return Ok(Vec::new());
        };
        let transcript = self.transcript_path(&rec, &folder.path);
        let mut out = self.adapter(rec.agent)?.subagent_timeline(
            asid,
            subagent,
            transcript.as_deref(),
            &self.config.env,
        );
        out.sort_by_key(|e| e.at);
        Ok(out)
    }

    pub fn timeline(&self, id: SessionId) -> Result<Vec<TimelineEntry>> {
        let rec = self.record(id)?;
        let folder = self.db().folder(rec.folder_id)?;
        let mut out: Vec<TimelineEntry> = self
            .db()
            .hook_events(id)?
            .into_iter()
            .filter(|e| !matches!(e.event.as_str(), "PreToolUse" | "UserPromptSubmit"))
            .map(|HookEventRow { at, event, summary }| TimelineEntry {
                at,
                kind: TimelineKind::Hook,
                text: format!("{event}: {summary}"),
            })
            .collect();
        if let Some(path) = self.transcript_path(&rec, &folder.path) {
            out.extend(match rec.agent {
                AgentId::Claude => transcript::claude_timeline(&path),
                AgentId::Codex => transcript::codex_timeline(&path),
                _ => Vec::new(),
            });
        } else {
            // Without a transcript, the hook events are all we have; include the tool calls.
            out.extend(
                self.db()
                    .hook_events(id)?
                    .into_iter()
                    .filter(|e| matches!(e.event.as_str(), "PreToolUse" | "UserPromptSubmit"))
                    .map(|e| TimelineEntry {
                        at: e.at,
                        kind: if e.event == "PreToolUse" {
                            TimelineKind::Tool
                        } else {
                            TimelineKind::Prompt
                        },
                        text: e.summary,
                    }),
            );
        }
        out.sort_by_key(|e| e.at);
        Ok(out)
    }

    /// Sessions of installed agents running right now, outside Termlab's own ptys.
    pub fn external_sessions(&self) -> Vec<External> {
        let installed = self.installed_map();
        let folders = self.list_folders().unwrap_or_default();
        let mut out = Vec::new();
        for adapter in &self.config.adapters {
            let Some(inst) = installed.get(&adapter.id()) else {
                continue;
            };
            for running in adapter.running(&inst.path, &self.config.env) {
                let owned = self
                    .db()
                    .session_by_agent_id(running.agent, &running.agent_session_id)
                    .ok()
                    .flatten();
                // Sessions Termlab itself runs (started, or already attached) aren't external.
                if let Some(rec) = &owned
                    && self.is_live(rec.id)
                    && rec.origin != SessionOrigin::Attached
                {
                    continue;
                }
                let lr_ok = adapter
                    .attach(
                        &running,
                        &LaunchRequest {
                            session_id: SessionId::new(),
                            program: inst.path.clone(),
                            cwd: running.cwd.clone(),
                            model: None,
                            permission: PermissionPreset::AskFirst,
                            first_message: None,
                            hooks: None,
                        },
                    )
                    .is_some();
                out.push(External {
                    folder_id: folders
                        .iter()
                        .find(|f| crate::adapter::same_dir(&f.path, &running.cwd))
                        .map(|f| f.id),
                    attachable: lr_ok,
                    termlab_session: owned.filter(|r| self.is_live(r.id)).map(|r| r.id),
                    running,
                });
            }
        }
        out.sort_by(|a, b| b.running.started_at.cmp(&a.running.started_at));
        out
    }

    /// Past sessions recorded by installed agents: in one folder, in all added folders, or
    /// (`everywhere`) in any folder on this machine.
    pub fn discover(
        &self,
        folder_id: Option<FolderId>,
        everywhere: bool,
    ) -> Result<Vec<Discovered>> {
        let folders: Vec<Folder> = self.list_folders()?;
        let installed = self.installed_map();
        let owned: HashMap<(AgentId, String), SessionId> = self
            .db()
            .list_sessions()?
            .into_iter()
            .filter_map(|s| Some(((s.agent, s.agent_session_id?), s.id)))
            .collect();
        let running: HashSet<(AgentId, String)> = self
            .external_sessions()
            .into_iter()
            .map(|e| (e.running.agent, e.running.agent_session_id))
            .collect();
        let adapters = self
            .config
            .adapters
            .iter()
            .filter(|a| installed.contains_key(&a.id()));
        let mut found: Vec<PastSession> = Vec::new();
        for adapter in adapters {
            if everywhere && folder_id.is_none() {
                found.extend(adapter.discover_all(&self.config.env));
            } else {
                for f in folders
                    .iter()
                    .filter(|f| folder_id.is_none_or(|id| id == f.id))
                {
                    found.extend(adapter.discover(&f.path, &self.config.env));
                }
            }
        }
        let mut out: Vec<Discovered> = found
            .into_iter()
            .map(|past| {
                let key = (past.agent, past.agent_session_id.clone());
                Discovered {
                    folder_id: folders
                        .iter()
                        .find(|f| crate::adapter::same_dir(&f.path, &past.cwd))
                        .map(|f| f.id),
                    termlab_session: owned.get(&key).copied(),
                    running_elsewhere: running.contains(&key),
                    past,
                }
            })
            .collect();
        out.sort_by(|a, b| b.past.last_activity_at.cmp(&a.past.last_activity_at));
        out.truncate(1000);
        Ok(out)
    }

    /// Adds past agent sessions to Termlab as ended sessions (with "Resume here"), adding
    /// their folders as needed. Sessions Termlab already tracks are left alone.
    pub fn import(&self, items: &[PastSession]) -> Result<ImportSummary> {
        let mut summary = ImportSummary::default();
        let mut folders = self.list_folders()?;
        for past in items {
            if self
                .db()
                .session_by_agent_id(past.agent, &past.agent_session_id)?
                .is_some()
            {
                summary.already_present += 1;
                continue;
            }
            let folder = match folders
                .iter()
                .find(|f| crate::adapter::same_dir(&f.path, &past.cwd))
            {
                Some(f) => f.clone(),
                None if past.cwd.is_dir() => {
                    let f = self.add_folder(&past.cwd)?;
                    summary.folders_added += 1;
                    folders.push(f.clone());
                    f
                }
                None => {
                    summary.skipped += 1;
                    continue;
                }
            };
            let record = SessionRecord {
                id: SessionId::new(),
                folder_id: folder.id,
                agent: past.agent,
                title: past.title.clone(),
                model: None,
                permission_mode: None,
                isolation: Isolation::Checkout,
                agent_session_id: Some(past.agent_session_id.clone()),
                status: SessionStatus::Done,
                exit_code: None,
                created_at: past.last_activity_at,
                last_activity_at: past.last_activity_at,
                billing: self.billing(past.agent),
                start_commit: None,
                transcript_path: past
                    .path
                    .is_file()
                    .then(|| past.path.to_string_lossy().into_owned()),
                notify: true,
                origin: SessionOrigin::Imported,
                term_size: None,
                color: None,
            };
            self.db().insert_session(&record)?;
            summary.imported += 1;
            self.emit(CoreEvent::SessionUpdated {
                session: Box::new(self.view(record)),
            });
        }
        Ok(summary)
    }

    /// Attaches a Termlab terminal to a background session the agent runs elsewhere.
    pub fn attach_external(
        self: &Arc<Self>,
        agent: AgentId,
        agent_session_id: &str,
        size: Option<TermSize>,
    ) -> Result<SessionView> {
        let ext = self
            .external_sessions()
            .into_iter()
            .find(|e| e.running.agent == agent && e.running.agent_session_id == agent_session_id)
            .ok_or_else(|| CoreError::InvalidInput("that session isn't running any more".into()))?;
        if let Some(id) = ext.termlab_session {
            return self.session(id);
        }
        if !ext.attachable {
            return Err(CoreError::InvalidInput(
                "that session is open in another terminal window and can't be attached to".into(),
            ));
        }
        let folder_id = match ext.folder_id {
            Some(id) => id,
            None => self.add_folder(&ext.running.cwd)?.id,
        };
        let title = ext
            .running
            .name
            .clone()
            .unwrap_or_else(|| format!("{} background session", agent.display_name()));
        self.start_session(
            &StartRequest {
                folder_id,
                agent,
                model: None,
                permission: PermissionPreset::AskFirst,
                first_message: None,
                size,
                notify: true,
                title: Some(title),
                mode: StartMode::Attach {
                    running: ext.running,
                },
            },
            None,
        )
    }

    /// Refuses to resume a conversation the agent is running elsewhere right now: two
    /// clients on one conversation would interleave.
    fn ensure_not_running_elsewhere(&self, agent: AgentId, agent_session_id: &str) -> Result<()> {
        let elsewhere = self.external_sessions().into_iter().find(|e| {
            e.running.agent == agent
                && e.running.agent_session_id == agent_session_id
                && e.termlab_session.is_none()
        });
        match elsewhere {
            Some(e) if e.attachable => Err(CoreError::InvalidInput(
                "this session is running in the background right now — attach to it instead".into(),
            )),
            Some(_) => Err(CoreError::InvalidInput(
                "this session is open in another terminal window right now".into(),
            )),
            None => Ok(()),
        }
    }

    // ---- hooks ----------------------------------------------------------------

    /// Listens for `termlab-hook` messages on a Unix socket.
    pub fn serve_hooks(self: &Arc<Self>, path: &Path) -> Result<()> {
        let _ = std::fs::remove_file(path);
        let listener = {
            let _guard = self.rt.enter();
            tokio::net::UnixListener::bind(path)?
        };
        restrict_permissions(path);
        // Messages are applied one at a time in arrival order: an agent's hooks fire in
        // sequence (PreToolUse, then PermissionRequest…) and must not overtake each other.
        // `termlab-hook` writes and disconnects at once, so reading in turn is cheap.
        let weak = Arc::downgrade(self);
        self.rt.spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    continue;
                };
                let mut buf = Vec::new();
                let read = tokio::time::timeout(
                    Duration::from_secs(2),
                    (&mut stream)
                        .take(MAX_MESSAGE_BYTES as u64)
                        .read_to_end(&mut buf),
                )
                .await;
                if !matches!(read, Ok(Ok(_))) {
                    continue;
                }
                let Some(this) = weak.upgrade() else { return };
                match hooks::parse_message(&buf) {
                    Some(msg) => {
                        let _ = tokio::task::spawn_blocking(move || this.apply_hook(&msg)).await;
                    }
                    None => tracing::debug!("ignored malformed hook message"),
                }
            }
        });
        Ok(())
    }

    /// Applies a hook event to its session. Public for tests and alternative transports.
    pub fn apply_hook(&self, msg: &HookMessage) {
        let Ok(rec) = self.record(msg.session) else {
            tracing::debug!(session = %msg.session, "hook for an unknown session");
            return;
        };
        if rec.agent != msg.agent {
            return;
        }
        // Claude Code's status line: it carries the plan limits, runs often, and isn't
        // something the session did, so it stays out of the status and the Timeline.
        if msg.event.starts_with("StatusLine") {
            let report = PlanReport {
                windows: crate::adapter::claude_plan_windows(&msg.payload),
                at: now_ms(),
                ..PlanReport::default()
            };
            self.record_plan_usage(msg.agent, report, true);
            return;
        }
        let mut effect = hooks::interpret(msg);
        let now = now_ms();
        // Codex's `notify` also fires for side threads (e.g. title generation). Only events
        // for the session's own thread count, and a thread id is only adopted if Codex
        // wrote a rollout for it (that's the id `codex resume` takes).
        if rec.agent == AgentId::Codex
            && let Some(tid) = effect.agent_session_id.clone()
        {
            match rec.agent_session_id.as_deref() {
                Some(known) if known != tid => return,
                Some(_) => {}
                None => {
                    if let Some(path) = codex_rollout_path(&self.config.env, &tid) {
                        effect.transcript_path = Some(path.to_string_lossy().into_owned());
                    } else {
                        effect.agent_session_id = None;
                    }
                }
            }
        }
        {
            let db = self.db();
            if let Some(asid) = &effect.agent_session_id
                && rec.agent_session_id.as_deref() != Some(asid)
            {
                let _ = db.set_agent_session_id(rec.id, asid);
            }
            if let Some(tp) = &effect.transcript_path
                && rec.transcript_path.as_deref() != Some(tp)
            {
                let _ = db.set_transcript_path(rec.id, tp);
            }
            if let Some(summary) = &effect.summary {
                let _ = db.add_hook_event(rec.id, now, &msg.event, summary);
            }
            let _ = db.touch(rec.id, now);
        }
        {
            let mut meta = self.meta.lock().expect("meta lock");
            let m = meta.entry(rec.id).or_default();
            if let Some(s) = effect.status {
                m.hook_status = Some(s);
                m.hook_at = now;
            }
            if let Some(detail) = effect.approval_detail {
                m.approval_detail = detail;
            }
            // Re-read usage soon: a turn just progressed.
            m.usage_checked_at = 0;
        }
        self.evaluate(rec.id, now);
    }

    // ---- status monitor -------------------------------------------------------

    /// Recomputes a live session's status, preview, approval and usage; emits on change.
    fn evaluate(&self, id: SessionId, now: Millis) {
        let Ok(pty) = self.live(id) else { return };
        let Ok(rec) = self.record(id) else { return };
        let Ok(adapter) = self.adapter(rec.agent) else {
            return;
        };
        let screen = pty.screen();
        // Activity = the visible text changed, not merely bytes arrived: several TUIs
        // (Codex, Vibe) redraw an unchanged screen while they wait.
        let screen_hash = crate::status::activity_hash(&screen.rows);
        let last_output = {
            let mut meta = self.meta.lock().expect("meta lock");
            let m = meta.entry(id).or_default();
            if m.screen_hash != Some(screen_hash) {
                m.screen_hash = Some(screen_hash);
                let out = pty.last_output_at();
                let typed = pty.last_input_at();
                // The screen changing just after a keystroke is the agent echoing what you
                // type, not the agent working — unless it was already busy.
                // (Either order: when you type quickly, the next keystroke can arrive after the
                // previous one's echo but before this check.)
                let echo = typed > 0
                    && (out - typed).abs() < TYPING_ECHO_MS
                    && now - m.changed_at >= status::RUNNING_WINDOW_MS;
                if !echo {
                    m.changed_at = out.max(m.changed_at).min(now);
                    if m.changed_at == 0 {
                        m.changed_at = now;
                    }
                }
            }
            m.changed_at
        };
        let heuristic = heuristic_status(now, last_output, &screen, &adapter.status_hints());
        let prompt = parse_prompt(&screen.rows);

        // Slow work outside the meta lock.
        let (need_usage, need_id) = {
            let meta = self.meta.lock().expect("meta lock");
            let m = meta.get(&id);
            (
                m.is_none_or(|m| now - m.usage_checked_at >= USAGE_INTERVAL_MS),
                rec.agent_session_id.is_none()
                    && m.is_none_or(|m| now - m.id_checked_at >= ID_CHECK_INTERVAL_MS),
            )
        };
        let folder = self.db().folder(rec.folder_id).ok();
        // Agents that pick their own session id (Codex, Vibe): find it on disk, so the
        // session can be resumed.
        if need_id && let Some(folder) = &folder {
            let claimed: HashSet<String> = self
                .db()
                .list_sessions()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s.agent == rec.agent)
                .filter_map(|s| s.agent_session_id)
                .collect();
            if let Some((asid, path)) = adapter.find_started_session(
                &folder.path,
                rec.created_at,
                &claimed,
                &self.config.env,
            ) {
                let db = self.db();
                let _ = db.set_agent_session_id(id, &asid);
                if let Some(path) = path {
                    let _ = db.set_transcript_path(id, &path.to_string_lossy());
                }
            }
        }
        let usage = if need_usage {
            folder.as_ref().and_then(|f| self.read_usage(&rec, &f.path))
        } else {
            None
        };
        let subagents = need_usage
            .then(|| self.read_subagents(&rec, folder.as_ref().map(|f| f.path.as_path()), now));
        // A placeholder title ("New Claude Code session") gives way to the agent's own
        // title for the conversation once it has one.
        let mut retitled = false;
        if need_usage
            && is_placeholder_title(&rec)
            && let Some(f) = &folder
            && let Some(title) = self.history_title(&rec, &f.path)
        {
            retitled = self.db().set_title(id, &title).is_ok();
        }
        // A rename or colour set inside the agent (Claude's /rename, /color) applies here too.
        if need_usage
            && let Some(f) = &folder
            && self.apply_agent_marks(id, &rec, &f.path)
        {
            retitled = true;
        }

        let mut changed = retitled;
        let mut new_status = None;
        {
            let mut meta = self.meta.lock().expect("meta lock");
            let m = meta.entry(id).or_default();
            if need_id {
                m.id_checked_at = now;
            }
            if need_usage {
                m.usage_checked_at = now;
                if usage.is_some() && usage != m.usage {
                    m.usage = usage;
                    changed = true;
                }
                if let Some(subagents) = subagents
                    && subagents != m.subagents
                {
                    m.subagents = subagents;
                    changed = true;
                }
            }
            // A hook's "Needs you" goes stale once the prompt has left the screen and the
            // agent has moved on (e.g. the user answered in the terminal).
            if m.hook_status == Some(SessionStatus::NeedsYou)
                && prompt.is_none()
                && now - m.hook_at > STALE_NEEDS_YOU_MS
                && last_output > m.hook_at
            {
                m.hook_status = None;
                m.approval_detail = None;
            }
            // A hook's "Idle" holds until the agent prints again: some agents report turn
            // ends for side work, and the screen then shows what really happens next.
            if m.hook_status == Some(SessionStatus::Idle)
                && last_output > m.hook_at + IDLE_HOOK_GRACE_MS
            {
                m.hook_status = None;
            }
            let status = m.hook_status.unwrap_or(heuristic);
            let approval = (status == SessionStatus::NeedsYou).then(|| match &prompt {
                Some(p) => Approval {
                    detail: m.approval_detail.clone(),
                    ..p.approval.clone()
                },
                None => Approval {
                    question: format!("{} is waiting for you", rec.agent.display_name()),
                    detail: m.approval_detail.clone(),
                    options: Vec::new(),
                },
            });
            let preview = screen.tail(PREVIEW_LINES);
            if m.status != Some(status) {
                if m.status.is_some() || status != rec.status {
                    new_status = Some(status);
                }
                m.status = Some(status);
                changed = true;
            }
            if m.preview != preview {
                m.preview = preview;
                changed = true;
            }
            if m.approval != approval {
                m.approval = approval;
                changed = true;
            }
        }
        if let Some(s) = new_status {
            let _ = self.db().set_status(id, s, now);
        }
        if changed {
            self.emit_session(id);
        }
    }

    fn save_live_scrollback(&self, id: SessionId, pty: &PtyProcess, now: Millis) {
        let due = {
            let mut meta = self.meta.lock().expect("meta lock");
            let m = meta.entry(id).or_default();
            let last = pty.last_output_at();
            if now - m.saved_at >= SCROLLBACK_SAVE_INTERVAL_MS && last != m.saved_output_at {
                m.saved_at = now;
                m.saved_output_at = last;
                true
            } else {
                false
            }
        };
        if due {
            let _ = pty.flush_scrollback(&self.config.dirs.scrollback_path(id));
        }
    }

    fn tick(&self) {
        let now = now_ms();
        let live: Vec<(SessionId, Arc<PtyProcess>)> = self
            .live
            .read()
            .expect("live lock")
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        for (id, pty) in live {
            self.evaluate(id, now);
            self.save_live_scrollback(id, &pty, now);
        }
    }

    /// Saves every live session's scrollback and stops them. Called when the daemon exits.
    pub fn shutdown(&self) {
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let live: Vec<_> = self.live.write().expect("live lock").drain().collect();
        for (id, pty) in live {
            if let Err(e) = pty.flush_scrollback(&self.config.dirs.scrollback_path(id)) {
                tracing::warn!(session = %id, error = %e, "could not save scrollback");
            }
            // Ended cleanly, and resumed automatically when the service starts again.
            let db = self.db();
            let _ = db.set_exited(id, SessionStatus::Done, None, now_ms());
            let _ = db.set_resume_on_start(id, true);
            drop(db);
            let _ = pty.kill();
        }
    }

    /// The visible screen of a live session, as plain text rows (for debugging tools).
    pub fn screen_text(&self, id: SessionId) -> Result<Vec<String>> {
        Ok(self.live(id)?.screen().rows)
    }

    pub fn live_count(&self) -> usize {
        self.live.read().expect("live lock").len()
    }
}

fn monitor_loop(weak: Weak<Termlab>) {
    loop {
        std::thread::sleep(MONITOR_INTERVAL);
        let Some(this) = weak.upgrade() else { return };
        this.tick();
    }
}

/// Owner-only access for sockets in the data dir.
pub fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

fn load_plan_usage(path: &Path) -> HashMap<AgentId, PlanUsage> {
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    // A list, one entry per agent (an earlier version stored a single object).
    let list: Vec<PlanUsage> = serde_json::from_slice(&bytes)
        .or_else(|_| serde_json::from_slice::<PlanUsage>(&bytes).map(|one| vec![one]))
        .unwrap_or_default();
    list.into_iter().map(|u| (u.agent, u)).collect()
}

/// Keeps the plan limits current: asks each agent that can report them when it's due.
fn plan_usage_loop(weak: Weak<Termlab>) {
    loop {
        let Some(this) = weak.upgrade() else { return };
        if this.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        this.refresh_plan_usage(false);
        drop(this);
        std::thread::sleep(Duration::from_secs(15));
    }
}

/// Titles Termlab makes up when it knows nothing better.
fn is_placeholder_title(rec: &SessionRecord) -> bool {
    let name = rec.agent.display_name();
    [
        format!("New {name} session"),
        format!("Resumed {name} session"),
        format!("Forked {name} session"),
        format!("{name} background session"),
    ]
    .contains(&rec.title)
}

fn default_title(req: &StartRequest) -> String {
    match req
        .first_message
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        Some(msg) => one_line(msg.lines().next().unwrap_or(msg), 80),
        None => match &req.mode {
            StartMode::New => format!("New {} session", req.agent.display_name()),
            StartMode::Resume { .. } => format!("Resumed {} session", req.agent.display_name()),
            StartMode::Fork { .. } => format!("Forked {} session", req.agent.display_name()),
            StartMode::Attach { .. } => format!("{} background session", req.agent.display_name()),
        },
    }
}
