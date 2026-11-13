//! Mistral Vibe (`vibe`). Verified against vibe 3.0.3.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use std::collections::HashSet;

use super::{
    AgentAdapter, CommandSpec, LaunchRequest, PastSession, PlanPoll, PlanReport, SUBAGENT_QUIET_MS,
    StatusHints, home, mtime_ms, one_line, parse_rfc3339_ms, same_dir,
};
use crate::env::UserEnv;
use crate::model::{AgentId, Millis, PermissionPreset, SubAgent};

const DISCOVERY_LIMIT: usize = 40;
const DISCOVER_ALL_LIMIT: usize = 200;

fn session_root(env: &UserEnv) -> Option<PathBuf> {
    env.get("VIBE_HOME")
        .map(PathBuf::from)
        .or_else(|| home(env).map(|h| h.join(".vibe")))
        .map(|v| v.join("logs").join("session"))
}

/// The plan Mistral reports for the signed-in account, from Vibe's own cache of its
/// `whoami` lookup (`~/.vibe/whoami_cache.json`: one entry per key, each with
/// `stored_at_timestamp` and a `payload` holding `plan_name` / `plan_type`). The newest
/// entry wins. Verified against vibe 3.0.3.
pub fn vibe_plan(env: &UserEnv) -> Option<String> {
    let home = env
        .get("VIBE_HOME")
        .map(PathBuf::from)
        .or_else(|| home(env).map(|h| h.join(".vibe")))?;
    let cache: Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("whoami_cache.json")).ok()?)
            .ok()?;
    let newest = cache.as_object()?.values().max_by_key(|e| {
        e.get("stored_at_timestamp")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    })?;
    let payload = newest.get("payload")?;
    let name = ["plan_name", "plan_type"].iter().find_map(|key| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })?;
    // "INDIVIDUAL" reads better as "Individual".
    let shouting = name.chars().all(|c| !c.is_lowercase());
    Some(if shouting {
        let lower = name.to_lowercase();
        let mut chars = lower.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => lower,
        }
    } else {
        name.to_string()
    })
}

/// One recorded Vibe conversation.
struct VibeSession {
    id: String,
    cwd: PathBuf,
    title: Option<String>,
    started_at: Millis,
    last_activity_at: Millis,
    dir: PathBuf,
    /// `parent_session_id`: set on a session another one started.
    parent: Option<String>,
    /// `end_time` is set.
    ended: bool,
}

/// Vibe keeps conversations in two layouts (verified against vibe 3.0.3):
/// - `~/.vibe/logs/session/unified/<session_id>/meta.json` (the current store), whose
///   `meta.json` has `session_id`, `start_time`, `environment.working_directory` and a
///   generated `title`;
/// - older `~/.vibe/logs/session/session_<date>_<time>_<id>/` folders with the same
///   `meta.json` fields (no title) and a `messages.jsonl`.
///
/// Vibe only writes a conversation once it has messages.
fn read_session(dir: &Path) -> Option<VibeSession> {
    let meta_path = dir.join("meta.json");
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(&meta_path).ok()?).ok()?;
    let id = meta.get("session_id")?.as_str()?.to_string();
    let cwd = PathBuf::from(
        meta.get("environment")?
            .get("working_directory")?
            .as_str()?,
    );
    let title = meta
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .map(String::from)
        .or_else(|| {
            // The older layout: the first user message.
            let f = std::fs::File::open(dir.join("messages.jsonl")).ok()?;
            BufReader::new(f)
                .lines()
                .map_while(Result::ok)
                .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
                .find(|v| v.get("role").and_then(Value::as_str) == Some("user"))
                .and_then(|v| v.get("content")?.as_str().map(String::from))
        });
    let started_at = meta
        .get("start_time")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_ms)
        .unwrap_or_else(|| mtime_ms(&meta_path));
    let last_activity_at = [
        mtime_ms(&meta_path),
        mtime_ms(&dir.join("CURRENT")),
        mtime_ms(&dir.join("messages.jsonl")),
        mtime_ms(dir),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    Some(VibeSession {
        id,
        cwd,
        title,
        started_at,
        last_activity_at,
        dir: dir.to_path_buf(),
        parent: meta
            .get("parent_session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from),
        ended: meta.get("end_time").is_some_and(|v| !v.is_null()),
    })
}

/// Every recorded conversation, newest first.
fn all_sessions(env: &UserEnv) -> Vec<VibeSession> {
    let Some(root) = session_root(env) else {
        return Vec::new();
    };
    let list = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    let mut dirs = list(&root.join("unified"));
    dirs.extend(list(&root));
    let mut out: Vec<VibeSession> = dirs
        .iter()
        .filter(|d| d.join("meta.json").is_file())
        .filter_map(|d| read_session(d))
        .collect();
    out.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
    out
}

fn vibe_sessions(env: &UserEnv, folder: Option<&Path>, limit: usize) -> Vec<PastSession> {
    all_sessions(env)
        .into_iter()
        .filter(|s| folder.is_none_or(|f| same_dir(&s.cwd, f)))
        .map(|s| PastSession {
            agent: AgentId::Vibe,
            title: one_line(s.title.as_deref().unwrap_or("Untitled Vibe session"), 100),
            agent_session_id: s.id,
            cwd: s.cwd,
            last_activity_at: s.last_activity_at,
            path: s.dir,
        })
        .take(limit)
        .collect()
}

pub struct VibeAdapter;

impl VibeAdapter {
    /// Built-in agent profiles: `ask` asks before every tool call, `accept-edits` lets
    /// file edits through, `auto-approve` approves everything.
    fn agent_profile(p: PermissionPreset) -> &'static str {
        match p {
            PermissionPreset::AskFirst => "ask",
            PermissionPreset::AutoEdit => "accept-edits",
            PermissionPreset::FullAuto => "auto-approve",
        }
    }
}

impl AgentAdapter for VibeAdapter {
    fn id(&self) -> AgentId {
        AgentId::Vibe
    }

    fn binary_name(&self) -> &'static str {
        "vibe"
    }

    fn api_key_vars(&self) -> &'static [&'static str] {
        &["MISTRAL_API_KEY"]
    }

    /// Vibe has no model flag: the model is `active_model` in `~/.vibe/config.toml`,
    /// which Termlab doesn't edit.
    fn model_flag(&self) -> bool {
        false
    }

    fn can_resume(&self) -> bool {
        true
    }

    /// `--resume <SESSION_ID>`; ids come from Vibe's session logs.
    fn resume(&self, agent_session_id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        let mut spec = self.launch(req);
        spec.args
            .splice(0..0, ["--resume".to_string(), agent_session_id.to_string()]);
        spec.agent_session_id = Some(agent_session_id.into());
        Some(spec)
    }

    fn discover(&self, folder: &Path, env: &UserEnv) -> Vec<PastSession> {
        vibe_sessions(env, Some(folder), DISCOVERY_LIMIT)
    }

    fn discover_all(&self, env: &UserEnv) -> Vec<PastSession> {
        vibe_sessions(env, None, DISCOVER_ALL_LIMIT)
    }

    /// Vibe picks its own session id; it's the newest conversation started in `folder`
    /// since the Termlab session began (Vibe writes it once the first message is sent).
    fn find_started_session(
        &self,
        folder: &Path,
        since: Millis,
        claimed: &HashSet<String>,
        env: &UserEnv,
    ) -> Option<(String, Option<PathBuf>)> {
        all_sessions(env)
            .into_iter()
            .filter(|s| s.started_at >= since - 5_000 && !claimed.contains(&s.id))
            .find(|s| same_dir(&s.cwd, folder))
            .map(|s| (s.id, None))
    }

    fn plan_usage_poll(&self) -> Option<PlanPoll> {
        // Reading Vibe's own cache file is cheap.
        Some(PlanPoll {
            every_ms: 60_000,
            only_while_live: false,
        })
    }

    /// Whether Vibe is connected and on which plan. Mistral gives Vibe no usage percentage
    /// (no limit lookup exists in Vibe; it only reacts when a request is refused).
    fn plan_usage(&self, _program: &Path, env: &UserEnv, _scratch: &Path) -> Option<PlanReport> {
        let plan = vibe_plan(env);
        let has_key = env
            .get("MISTRAL_API_KEY")
            .is_some_and(|k| !k.trim().is_empty());
        let connected = plan.is_some() || has_key;
        Some(PlanReport {
            windows: Vec::new(),
            at: crate::model::now_ms(),
            connected: Some(connected),
            plan,
            // Connected: the plan's name says it all. (Mistral gives Vibe no usage figure.)
            note: (!connected).then(|| "Not signed in. Run `vibe --setup` to connect.".to_string()),
        })
    }

    /// Sessions whose `meta.json` names this one as `parent_session_id` (vibe 3.0.3's format;
    /// no real example was available to check against).
    fn subagents(
        &self,
        agent_session_id: &str,
        _transcript: Option<&Path>,
        _since: Millis,
        now: Millis,
        env: &UserEnv,
    ) -> Vec<SubAgent> {
        all_sessions(env)
            .into_iter()
            .filter(|s| s.parent.as_deref() == Some(agent_session_id))
            .map(|s| SubAgent {
                name: s
                    .title
                    .map(|t| one_line(&t, 60))
                    .unwrap_or_else(|| "agent".into()),
                description: None,
                model: None,
                running: !s.ended && now - s.last_activity_at < SUBAGENT_QUIET_MS,
                started_at: Some(s.started_at),
                last_activity_at: s.last_activity_at,
                id: s.id,
            })
            .collect()
    }

    /// Vibe's generated title, from the conversation's `meta.json`.
    fn history_title(
        &self,
        agent_session_id: &str,
        _transcript: Option<&Path>,
        env: &UserEnv,
    ) -> Option<String> {
        let dir = session_root(env)?.join("unified").join(agent_session_id);
        read_session(&dir)?.title
    }

    fn launch(&self, req: &LaunchRequest) -> CommandSpec {
        let mut args = vec![
            "--agent".to_string(),
            Self::agent_profile(req.permission).into(),
        ];
        if let Some(msg) = req.first_message() {
            if msg.starts_with('-') {
                args.push("--".into());
            }
            args.push(msg.into());
        }
        CommandSpec::new(req, args, None)
    }

    /// Vibe's approval dialog is recognised from its option list; its footer reads
    /// "↑↓/jk navigate  Enter select  Esc reject". Bare words like "approve" must not be hints:
    /// the idle prompt's mode label reads "auto approve".
    fn status_hints(&self) -> StatusHints {
        StatusHints {
            needs_input: vec!["enter select  esc reject"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SessionId;

    #[test]
    fn reads_the_plan_from_the_whoami_cache() {
        let home = tempfile::tempdir().unwrap();
        let env = UserEnv::from_vars([("HOME", home.path().to_str().unwrap())]);
        assert_eq!(vibe_plan(&env), None);
        let report = VibeAdapter
            .plan_usage(Path::new("vibe"), &env, home.path())
            .unwrap();
        assert_eq!(report.connected, Some(false));

        std::fs::create_dir_all(home.path().join(".vibe")).unwrap();
        // Shaped like vibe 3.0.3's cache: the newest entry is the current account.
        std::fs::write(
            home.path().join(".vibe/whoami_cache.json"),
            r#"{"aaa":{"format":2,"stored_at_timestamp":100,"payload":{"plan_type":"chat","plan_name":"FREE"}},
                "bbb":{"format":2,"stored_at_timestamp":200,"payload":{"plan_type":"chat","plan_name":"INDIVIDUAL"}}}"#,
        )
        .unwrap();
        assert_eq!(vibe_plan(&env).as_deref(), Some("Individual"));
        let report = VibeAdapter
            .plan_usage(Path::new("vibe"), &env, home.path())
            .unwrap();
        assert_eq!(report.connected, Some(true));
        assert!(report.windows.is_empty());
        assert_eq!(report.note, None);
    }

    #[test]
    fn launch_args() {
        let req = LaunchRequest {
            session_id: SessionId::new(),
            program: "/bin/vibe".into(),
            cwd: "/src".into(),
            model: Some("ignored".into()),
            permission: PermissionPreset::AutoEdit,
            first_message: Some("dark mode".into()),
            hooks: None,
        };
        assert_eq!(
            VibeAdapter.launch(&req).args,
            ["--agent", "accept-edits", "dark mode"]
        );
        assert_eq!(
            VibeAdapter.resume("930605ce", &req).unwrap().args,
            [
                "--resume",
                "930605ce",
                "--agent",
                "accept-edits",
                "dark mode"
            ]
        );
    }

    #[test]
    fn discovers_session_logs() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".vibe/logs/session");
        for (dir, id, cwd, msg) in [
            (
                "session_20260302_082202_930605ce",
                "930605ce-08f1",
                "/w/aitest",
                "Hi",
            ),
            (
                "session_20260310_090000_aaaabbbb",
                "aaaabbbb-0000",
                "/w/other",
                "Add dark mode",
            ),
        ] {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("meta.json"),
                format!(r#"{{"session_id":"{id}","environment":{{"working_directory":"{cwd}"}}}}"#),
            )
            .unwrap();
            std::fs::write(
                d.join("messages.jsonl"),
                format!("{{\"role\":\"user\",\"content\":\"{msg}\"}}\n{{\"role\":\"assistant\",\"content\":\"ok\"}}\n"),
            )
            .unwrap();
        }
        let env = UserEnv::from_vars([("HOME", home.path().to_str().unwrap())]);
        let all = VibeAdapter.discover_all(&env);
        let got: Vec<_> = all
            .iter()
            .map(|s| (s.agent_session_id.as_str(), s.title.as_str()))
            .collect();
        assert_eq!(
            got,
            [("aaaabbbb-0000", "Add dark mode"), ("930605ce-08f1", "Hi")]
        );
        assert_eq!(VibeAdapter.discover(Path::new("/w/aitest"), &env).len(), 1);
    }
}
