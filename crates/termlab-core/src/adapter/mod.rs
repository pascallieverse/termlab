//! Agent adapters: how to find, launch, resume and discover each agent CLI.
//!
//! Every flag used here was checked against the installed CLI's `--help`.

mod antigravity;
mod claude;
mod codex;
mod vibe;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub use antigravity::AntigravityAdapter;
pub use claude::{
    ClaudeAdapter, claude_plan_windows, claude_project_dir, claude_renamed_title,
    claude_settings_json, claude_subagents, claude_transcript_marks, claude_transcript_title,
    claude_usage_windows, claude_user_status_line, encode_claude_path,
};
pub use codex::{CodexAdapter, codex_find_new_rollout, codex_plan_usage, codex_rollout_path};
pub use vibe::VibeAdapter;

pub(crate) use claude::user_prompt_text as claude_user_prompt;
pub(crate) use codex::parse_rfc3339_ms;

use crate::env::UserEnv;
use crate::model::{AgentId, Billing, Millis, PermissionPreset, SessionId, SubAgent, UsageWindow};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub agent: AgentId,
    pub path: PathBuf,
    pub version: Option<String>,
    /// How the agent's own login bills, ignoring API-key variables.
    pub login_billing: Billing,
}

/// What an adapter supports, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub agent: AgentId,
    pub name: String,
    pub binary: String,
    pub installed: Option<Installed>,
    /// Whether a model can be chosen per session.
    pub model_flag: bool,
    pub model_suggestions: Vec<String>,
    pub can_resume: bool,
    pub can_fork: bool,
    /// Hook-based status (otherwise screen heuristics only).
    pub hooks: bool,
    /// False when the flags couldn't be checked against an installed CLI.
    pub verified: bool,
}

/// Where Termlab's hook forwarder lives, for adapters that support hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookSetup {
    /// The `termlab-hook` binary.
    pub binary: PathBuf,
    /// A Termlab-owned Claude Code settings file registering the hooks.
    pub claude_settings: PathBuf,
}

/// What the user asked for, in agent-neutral terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub session_id: SessionId,
    /// Resolved binary (from `detect`).
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub permission: PermissionPreset,
    pub first_message: Option<String>,
    pub hooks: Option<HookSetup>,
}

impl LaunchRequest {
    fn model(&self) -> Option<&str> {
        self.model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
    }

    fn first_message(&self) -> Option<&str> {
        self.first_message
            .as_deref()
            .filter(|m| !m.trim().is_empty())
    }
}

/// How to run it: program, args, env *additions* and working directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    /// The agent's own session id, when the launch fixes it up front.
    pub agent_session_id: Option<String>,
}

impl CommandSpec {
    fn new(req: &LaunchRequest, args: Vec<String>, agent_session_id: Option<String>) -> Self {
        Self {
            program: req.program.clone(),
            args,
            env: vec![("TERMLAB_SESSION_ID".into(), req.session_id.to_string())],
            cwd: req.cwd.clone(),
            agent_session_id,
        }
    }

    /// The command line without the `cd`, as a user would type it.
    pub fn command_line(&self) -> String {
        let program = self
            .program
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.program.display().to_string());
        let mut out = shell_quote(&program);
        for a in &self.args {
            out.push(' ');
            out.push_str(&shell_quote(a));
        }
        out
    }

    /// `cd <folder> && <program> <args…>`. Uses the program's file name (it is on the
    /// user's PATH); env additions are internal plumbing and not shown.
    pub fn display(&self) -> String {
        format!(
            "cd {} && {}",
            shell_quote(&self.cwd.to_string_lossy()),
            self.command_line()
        )
    }
}

/// A session the agent recorded on disk, offered in the Resume tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastSession {
    pub agent: AgentId,
    pub agent_session_id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub last_activity_at: Millis,
    /// The transcript or rollout file it came from.
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunningKind {
    /// Managed by the agent in the background; can be attached to.
    Background,
    /// Open in some terminal window; can't be attached to.
    Interactive,
}

/// An agent session running right now outside Termlab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningSession {
    pub agent: AgentId,
    pub agent_session_id: String,
    /// What the agent's attach command takes.
    pub short_id: Option<String>,
    pub name: Option<String>,
    pub cwd: PathBuf,
    pub kind: RunningKind,
    /// The agent's own state, mapped to Termlab's statuses.
    pub status: Option<crate::model::SessionStatus>,
    pub started_at: Millis,
}

/// Screen patterns for the generic status heuristics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusHints {
    /// Case-insensitive substrings that, on the last screen lines of a quiet session,
    /// mean the agent is waiting for an approval.
    pub needs_input: Vec<&'static str>,
}

pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> AgentId;

    /// The binary name looked up on PATH.
    fn binary_name(&self) -> &'static str;

    /// Environment variables that switch this agent to API-key billing.
    fn api_key_vars(&self) -> &'static [&'static str];

    fn detect(&self, env: &UserEnv) -> Option<Installed> {
        let path = env.which(self.binary_name())?;
        let version = run_capture(&path, &["--version"], env).and_then(|o| parse_version(&o));
        let login_billing = self.login_billing(&path, env);
        Some(Installed {
            agent: self.id(),
            path,
            version,
            login_billing,
        })
    }

    /// Billing of the agent's own login (subscription vs API key), when it can tell us.
    fn login_billing(&self, _path: &Path, _env: &UserEnv) -> Billing {
        Billing::Unknown
    }

    fn info(&self, env: &UserEnv, installed: Option<Installed>) -> AgentInfo {
        AgentInfo {
            agent: self.id(),
            name: self.id().display_name().into(),
            binary: self.binary_name().into(),
            installed,
            model_flag: self.model_flag(),
            model_suggestions: self.model_suggestions(env),
            can_resume: self.can_resume(),
            can_fork: self.can_fork(),
            hooks: self.hooks(),
            verified: self.verified(),
        }
    }

    fn model_flag(&self) -> bool {
        true
    }

    fn model_suggestions(&self, _env: &UserEnv) -> Vec<String> {
        Vec::new()
    }

    fn can_resume(&self) -> bool {
        false
    }

    fn can_fork(&self) -> bool {
        false
    }

    fn hooks(&self) -> bool {
        false
    }

    fn verified(&self) -> bool {
        true
    }

    fn launch(&self, req: &LaunchRequest) -> CommandSpec;

    fn resume(&self, _agent_session_id: &str, _req: &LaunchRequest) -> Option<CommandSpec> {
        None
    }

    /// A new session continuing from `agent_session_id`, leaving the original untouched.
    fn fork(&self, _agent_session_id: &str, _req: &LaunchRequest) -> Option<CommandSpec> {
        None
    }

    fn discover(&self, _folder: &Path, _env: &UserEnv) -> Vec<PastSession> {
        Vec::new()
    }

    /// Past sessions in any folder, newest first.
    fn discover_all(&self, _env: &UserEnv) -> Vec<PastSession> {
        Vec::new()
    }

    /// For agents that choose their own session id: the session this agent started in
    /// `folder` at or after `since` that no other Termlab session has `claimed`. Returns the
    /// id and its transcript file, if it has one.
    fn find_started_session(
        &self,
        _folder: &Path,
        _since: Millis,
        _claimed: &std::collections::HashSet<String>,
        _env: &UserEnv,
    ) -> Option<(String, Option<PathBuf>)> {
        None
    }

    /// Sub-agents the agent started in this conversation, from its own files. `since` is when
    /// the session started (to skip older files), `now` the current time.
    fn subagents(
        &self,
        _agent_session_id: &str,
        _transcript: Option<&Path>,
        _since: Millis,
        _now: Millis,
        _env: &UserEnv,
    ) -> Vec<SubAgent> {
        Vec::new()
    }

    /// What one of the conversation's sub-agents did, from its own transcript.
    fn subagent_timeline(
        &self,
        _agent_session_id: &str,
        _subagent_id: &str,
        _transcript: Option<&Path>,
        _env: &UserEnv,
    ) -> Vec<crate::transcript::TimelineEntry> {
        Vec::new()
    }

    /// How to keep the plan's limits current for this agent, if it can report them.
    fn plan_usage_poll(&self) -> Option<PlanPoll> {
        None
    }

    /// The account's plan and its limits right now. `scratch` is a folder to run in. May
    /// start the agent's CLI, so it's called off the main paths. None: couldn't find out.
    fn plan_usage(&self, _program: &Path, _env: &UserEnv, _scratch: &Path) -> Option<PlanReport> {
        None
    }

    /// What the user set on the conversation inside the agent (Claude's `/rename` and
    /// `/color`), looked for in the transcript after byte `from`. Returns the offset to
    /// continue from next time and the newest of each found.
    fn transcript_marks(&self, _transcript: &Path, from: u64) -> (u64, TranscriptMarks) {
        (from, TranscriptMarks::default())
    }

    /// The agent's own title for a conversation, when it has one.
    fn history_title(
        &self,
        _agent_session_id: &str,
        _transcript: Option<&Path>,
        _env: &UserEnv,
    ) -> Option<String> {
        None
    }

    /// Sessions of this agent running right now (outside Termlab or not).
    fn running(&self, _program: &Path, _env: &UserEnv) -> Vec<RunningSession> {
        Vec::new()
    }

    /// A command that attaches to a running background session.
    fn attach(&self, _running: &RunningSession, _req: &LaunchRequest) -> Option<CommandSpec> {
        None
    }

    fn status_hints(&self) -> StatusHints {
        StatusHints::default()
    }
}

pub fn all_adapters() -> Vec<Box<dyn AgentAdapter>> {
    vec![
        Box::new(ClaudeAdapter),
        Box::new(CodexAdapter),
        Box::new(VibeAdapter),
        Box::new(AntigravityAdapter),
    ]
}

/// Runs `<binary> <args>` with the user's environment and returns stdout+stderr, or
/// `None` on failure or after 5 seconds.
pub(crate) fn run_capture(path: &Path, args: &[&str], env: &UserEnv) -> Option<String> {
    let mut child = Command::new(path)
        .args(args)
        .env_clear()
        .envs(env.vars())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    while child.try_wait().ok()?.is_none() {
        if start.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut out = out_thread.join().unwrap_or_default();
    out.push_str(&err_thread.join().unwrap_or_default());
    Some(out)
}

pub(crate) fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .flat_map(|l| l.split_whitespace())
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.contains('.') && t.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

/// The user's home directory, from their environment.
pub(crate) fn home(env: &UserEnv) -> Option<PathBuf> {
    env.get("HOME").map(PathBuf::from)
}

/// What an agent's service says about the account: whether it's signed in, the plan, and
/// how much of each limit is used (where the service reports that).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlanReport {
    pub windows: Vec<UsageWindow>,
    /// When this was measured.
    pub at: Millis,
    pub connected: Option<bool>,
    pub plan: Option<String>,
    pub note: Option<String>,
}

/// How often to ask an agent for its plan limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanPoll {
    pub every_ms: Millis,
    /// Asking starts the agent's CLI: only do it while one of its sessions is running (or
    /// when the user asks).
    pub only_while_live: bool,
}

/// Things a user sets on a conversation from inside the agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptMarks {
    /// A new name (`/rename`).
    pub title: Option<String>,
    /// A new colour (`/color`), as the agent wrote it; "default" clears it.
    pub color: Option<String>,
}

/// A sub-agent whose transcript hasn't ended still counts as working for this long after
/// its last write (a long tool call writes nothing while it runs).
pub(crate) const SUBAGENT_QUIET_MS: Millis = 10 * 60 * 1000;

/// The last complete JSON line of a JSONL file, reading only its tail.
pub(crate) fn last_json_line(path: &Path) -> Option<serde_json::Value> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 256 * 1024;
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.split('\n').collect();
    // Unless the tail starts at the beginning of the file, its first piece is a partial line.
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines
        .into_iter()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .find_map(|l| serde_json::from_str(l).ok())
}

pub(crate) fn mtime_ms(path: &Path) -> Millis {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as Millis)
        .unwrap_or(0)
}

/// Same directory, comparing canonical paths when possible.
pub(crate) fn same_dir(a: &Path, b: &Path) -> bool {
    let ca = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let cb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Shortens a title to one line of at most `max` characters.
pub(crate) fn one_line(s: &str, max: usize) -> String {
    let line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max - 1).collect::<String>())
    } else {
        line
    }
}

/// POSIX shell quoting for display.
pub fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// TOML basic-string quoting (for Codex `-c key=value` overrides).
pub(crate) fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        assert_eq!(
            parse_version("2.1.322 (Claude Code)\n").as_deref(),
            Some("2.1.322")
        );
        assert_eq!(
            parse_version("codex-cli 0.167.0").as_deref(),
            Some("0.167.0")
        );
        assert_eq!(parse_version("vibe 3.0.3").as_deref(), Some("3.0.3"));
        assert_eq!(parse_version("1.5.6").as_deref(), Some("1.5.6"));
        assert_eq!(parse_version("aider v0.86.1").as_deref(), Some("0.86.1"));
        assert_eq!(parse_version("nothing here"), None);
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("--model"), "--model");
        assert_eq!(shell_quote("/a/b c"), "'/a/b c'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("~/x"), "'~/x'");
        assert_eq!(toml_string(r#"/a b/"q"\"#), r#""/a b/\"q\"\\""#);
    }

    #[test]
    fn one_line_titles() {
        assert_eq!(one_line("fix\n  the   tests", 80), "fix the tests");
        assert_eq!(one_line("abcdef", 4), "abc…");
    }
}
