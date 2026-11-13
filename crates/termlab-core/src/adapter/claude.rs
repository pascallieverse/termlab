//! Claude Code (`claude`). Verified against Claude Code 2.1.322.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    AgentAdapter, CommandSpec, LaunchRequest, PastSession, PlanPoll, PlanReport, RunningKind,
    RunningSession, SUBAGENT_QUIET_MS, StatusHints, TranscriptMarks, home, last_json_line,
    mtime_ms, one_line, parse_rfc3339_ms, run_capture, shell_quote,
};
use crate::env::UserEnv;
use crate::model::{
    AgentId, Billing, Millis, PermissionPreset, SessionStatus, SubAgent, UsageWindow,
};

pub struct ClaudeAdapter;

/// Hook events Termlab registers. All are present in Claude Code 2.1.322's hook schema.
const HOOK_EVENTS: &[(&str, bool)] = &[
    // (event, takes a tool matcher)
    ("SessionStart", false),
    ("UserPromptSubmit", false),
    ("PreToolUse", true),
    ("PostToolUse", true),
    ("PermissionRequest", true),
    ("Notification", false),
    ("Stop", false),
    ("SessionEnd", false),
];

/// Read at most this much of a transcript when discovering sessions.
const DISCOVERY_READ_BYTES: u64 = 512 * 1024;
const DISCOVERY_LIMIT: usize = 40;
const DISCOVER_ALL_LIMIT: usize = 400;

impl ClaudeAdapter {
    /// `--permission-mode` choices in 2.1.322: acceptEdits, auto, bypassPermissions,
    /// manual, dontAsk, plan. `manual` is the new name for the old `default`.
    fn permission_mode(p: PermissionPreset) -> &'static str {
        match p {
            PermissionPreset::AskFirst => "manual",
            PermissionPreset::AutoEdit => "acceptEdits",
            PermissionPreset::FullAuto => "bypassPermissions",
        }
    }

    fn common_args(req: &LaunchRequest, args: &mut Vec<String>) {
        if let Some(model) = req.model() {
            args.extend(["--model".into(), model.into()]);
        }
        args.extend([
            "--permission-mode".into(),
            Self::permission_mode(req.permission).into(),
        ]);
        if let Some(h) = &req.hooks {
            // A Termlab-owned settings file; ~/.claude/settings.json is never touched.
            args.extend([
                "--settings".into(),
                h.claude_settings.to_string_lossy().into_owned(),
            ]);
        }
    }

    fn push_prompt(req: &LaunchRequest, args: &mut Vec<String>) {
        if let Some(msg) = req.first_message() {
            if msg.starts_with('-') {
                args.push("--".into());
            }
            args.push(msg.into());
        }
    }
}

/// The settings JSON registering Termlab's hooks. Each hook runs
/// `termlab-hook claude <Event>`, which forwards the payload to Termlab and exits.
pub fn claude_settings_json(hook_binary: &Path, user_status_line: Option<&str>) -> Value {
    let hook = shell_quote(&hook_binary.to_string_lossy());
    // The status line is how Claude Code hands out the plan's rate-limit usage. Without a
    // status line of the user's own, the hook prints a short usage summary as the line.
    // With one, the hook only listens and the user's command still draws the line.
    let status_command = match user_status_line {
        Some(theirs) => format!(
            "input=$(cat); printf '%s' \"$input\" | {hook} claude StatusLineQuiet; printf '%s' \"$input\" | {{ {theirs}\n}}"
        ),
        None => format!("{hook} claude StatusLine"),
    };
    let mut hooks = serde_json::Map::new();
    for (event, matcher) in HOOK_EVENTS {
        let command = format!(
            "{} claude {event}",
            shell_quote(&hook_binary.to_string_lossy())
        );
        let handler = json!({ "type": "command", "command": command, "timeout": 5 });
        let group = if *matcher {
            json!({ "matcher": "*", "hooks": [handler] })
        } else {
            json!({ "hooks": [handler] })
        };
        hooks.insert((*event).into(), json!([group]));
    }
    json!({
        "hooks": hooks,
        "statusLine": { "type": "command", "command": status_command },
    })
}

/// The plan limits in Claude Code's structured usage report (the answer to a `get_usage`
/// control request; the same data its `/usage` screen shows). `rate_limits.limits[]` lists
/// each window, including per-model weekly ones:
/// `{"kind":"weekly_scoped","percent":25,"resets_at":"…","scope":{"model":{"display_name":"Fable"}}}`.
/// Experimental on Claude's side, so unknown kinds are kept under their own name. Returns
/// None when the answer isn't a usage report, an empty list when plan limits don't apply.
pub fn claude_usage_windows(control_response: &Value) -> Option<Vec<UsageWindow>> {
    let outer = control_response.get("response")?;
    let body = outer.get("response").unwrap_or(outer);
    let Some(limits) = body.get("rate_limits").filter(|v| v.is_object()) else {
        // `rate_limits: null`: API-key billing and the like.
        return body.get("rate_limits_available").map(|_| Vec::new());
    };
    let reset = |v: &Value| {
        v.get("resets_at")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms)
    };
    let mut windows = Vec::new();
    for item in limits
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(kind), Some(percent)) = (
            item.get("kind").and_then(Value::as_str),
            item.get("percent").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let scope = item.get("scope");
        let scope_name = ["model", "surface"].iter().find_map(|key| {
            scope?
                .get(key)?
                .get("display_name")?
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(String::from)
        });
        let weekly = item.get("group").and_then(Value::as_str) == Some("weekly")
            || kind.starts_with("weekly");
        let (id, label) = match (kind, weekly, scope_name) {
            ("weekly_all", _, _) => ("seven_day".to_string(), None),
            (_, true, Some(name)) => (
                format!("seven_day:{name}"),
                Some(format!("Current week ({name})")),
            ),
            (k, _, _) if k.contains("five_hour") || k.starts_with("session") => {
                ("five_hour".to_string(), None)
            }
            (k, _, name) => {
                let words = k.replace('_', " ");
                let label = match name {
                    Some(n) => format!("{words} ({n})"),
                    None => words,
                };
                (k.to_string(), Some(label))
            }
        };
        windows.push(UsageWindow {
            id,
            label,
            used_percentage: percent,
            resets_at: reset(item),
        });
    }
    // An older report without `limits[]`: the fixed windows.
    if windows.is_empty() {
        for id in ["five_hour", "seven_day"] {
            if let Some(w) = limits.get(id).filter(|w| w.is_object())
                && let Some(percent) = w.get("utilization").and_then(Value::as_f64)
            {
                windows.push(UsageWindow {
                    id: id.to_string(),
                    label: None,
                    used_percentage: percent,
                    resets_at: reset(w),
                });
            }
        }
    }
    Some(windows)
}

/// Asks Claude Code for its usage report: starts it in print mode with the stream-json
/// protocol, sends one `get_usage` control request and reads the answer. No prompt is sent,
/// so it uses none of the plan. Hooks, MCP servers and session saving are off for this run
/// (`--bare` isn't used: it also turns off the subscription login).
fn claude_fetch_usage(program: &Path, env: &UserEnv, scratch: &Path) -> Option<PlanReport> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new(program)
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--settings",
            r#"{"disableAllHooks":true}"#,
        ])
        .current_dir(scratch)
        .env_clear()
        .envs(env.for_spawn(&[]))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let answer = (|| {
        let mut stdin = child.stdin.take()?;
        stdin
            .write_all(b"{\"type\":\"control_request\",\"request_id\":\"termlab-usage\",\"request\":{\"subtype\":\"get_usage\"}}\n")
            .ok()?;
        stdin.flush().ok()?;
        let stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.contains("\"control_response\"") {
                    let _ = tx.send(line);
                    break;
                }
            }
        });
        let line = rx.recv_timeout(Duration::from_secs(20)).ok()?;
        // Closing its input (dropping `stdin`) ends the run.
        serde_json::from_str::<Value>(&line).ok()
    })();
    let deadline = Instant::now() + Duration::from_secs(3);
    while matches!(child.try_wait(), Ok(None)) {
        if Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.wait();
    claude_usage_report(&answer?)
}

/// Claude Code's usage report as a plan report: the plan's name (`subscription_type`) and
/// its limits; when plan limits don't apply (API-key billing and the like), a note says so.
pub fn claude_usage_report(control_response: &Value) -> Option<PlanReport> {
    let windows = claude_usage_windows(control_response)?;
    let outer = control_response.get("response")?;
    let body = outer.get("response").unwrap_or(outer);
    let plan = body
        .get("subscription_type")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
        .map(String::from);
    let limited = body.get("rate_limits_available").and_then(Value::as_bool) != Some(false);
    Some(PlanReport {
        note: (windows.is_empty() && !limited).then(|| {
            "Plan limits don't apply to this account (billed to an API key or another provider)."
                .to_string()
        }),
        windows,
        at: crate::model::now_ms(),
        connected: Some(true),
        plan,
    })
}

/// The command of the status line the user configured for Claude Code themselves, if any
/// (read from their settings; never written).
pub fn claude_user_status_line(env: &UserEnv) -> Option<String> {
    let dir = claude_config_dir(env)?;
    ["settings.local.json", "settings.json"]
        .iter()
        .filter_map(|name| std::fs::read_to_string(dir.join(name)).ok())
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .find_map(|v| {
            let line = v.get("statusLine")?;
            (line.get("type").and_then(Value::as_str) == Some("command"))
                .then(|| {
                    line.get("command")?
                        .as_str()
                        .map(str::trim)
                        .map(String::from)
                })
                .flatten()
                .filter(|c| !c.is_empty())
        })
}

/// The plan limits in Claude Code's status-line input: `rate_limits` maps each window
/// (`five_hour`, `seven_day`, `spend_limit`, …) to `used_percentage` and `resets_at` (Unix
/// seconds). Present for claude.ai subscribers after a session's first response.
pub fn claude_plan_windows(status_line_input: &Value) -> Vec<UsageWindow> {
    let Some(limits) = status_line_input
        .get("rate_limits")
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut windows: Vec<UsageWindow> = limits
        .iter()
        .filter_map(|(id, w)| {
            Some(UsageWindow {
                id: id.clone(),
                label: None,
                used_percentage: w.get("used_percentage")?.as_f64()?,
                resets_at: w
                    .get("resets_at")
                    .and_then(Value::as_i64)
                    .map(|secs| secs * 1000),
            })
        })
        .collect();
    // Shortest window first: five_hour, then seven_day…, then anything else.
    let rank = |id: &str| match id {
        "five_hour" => 0,
        "seven_day" => 1,
        id if id.starts_with("seven_day") => 2,
        _ => 3,
    };
    windows.sort_by(|a, b| rank(&a.id).cmp(&rank(&b.id)).then_with(|| a.id.cmp(&b.id)));
    windows
}

/// Claude Code's project directory name: every character that isn't an ASCII letter or
/// digit becomes `-` (verified against real directories on this machine).
pub fn encode_claude_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn claude_config_dir(env: &UserEnv) -> Option<PathBuf> {
    env.get("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| home(env).map(|h| h.join(".claude")))
}

pub fn claude_project_dir(folder: &Path, env: &UserEnv) -> Option<PathBuf> {
    Some(
        claude_config_dir(env)?
            .join("projects")
            .join(encode_claude_path(folder)),
    )
}

/// Top-level `.jsonl` transcripts in a project directory, with their mtimes.
fn jsonl_files(dir: &Path) -> Vec<(i64, PathBuf)> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .map(|p| (mtime_ms(&p), p))
                .collect()
        })
        .unwrap_or_default()
}

/// Past sessions from transcript files (newest first). Without `fallback_cwd`, files whose
/// transcript has no `cwd` are skipped.
fn sessions_from_files(
    mut files: Vec<(i64, PathBuf)>,
    fallback_cwd: Option<&Path>,
    limit: usize,
) -> Vec<PastSession> {
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .filter_map(|(mtime, path)| {
            let id = path.file_stem()?.to_string_lossy().into_owned();
            let (title, cwd) = read_transcript_head(&path)?;
            Some(PastSession {
                agent: AgentId::Claude,
                agent_session_id: id,
                title,
                cwd: cwd.or_else(|| fallback_cwd.map(Path::to_path_buf))?,
                last_activity_at: mtime,
                path,
            })
        })
        .take(limit)
        .collect()
}

/// Parses `claude agents --json`: `[{"id","sessionId","cwd","kind","name","state",…}]`.
/// `state` is `working`, `blocked` (waiting on the user) or `done`.
pub(crate) fn parse_agents_json(out: &str) -> Vec<RunningSession> {
    let start = out.find('[').unwrap_or(0);
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&out[start..]) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|x| {
            let s = |k: &str| x.get(k).and_then(Value::as_str);
            Some(RunningSession {
                agent: AgentId::Claude,
                agent_session_id: s("sessionId")?.to_string(),
                short_id: s("id").map(String::from),
                name: s("name").filter(|n| !n.is_empty()).map(String::from),
                cwd: PathBuf::from(s("cwd")?),
                kind: if s("kind") == Some("background") {
                    RunningKind::Background
                } else {
                    RunningKind::Interactive
                },
                status: match s("state").or(s("status")) {
                    Some("working" | "busy") => Some(SessionStatus::Running),
                    Some("blocked") => Some(SessionStatus::NeedsYou),
                    Some("done" | "idle") => Some(SessionStatus::Idle),
                    _ => None,
                },
                started_at: x.get("startedAt").and_then(Value::as_i64).unwrap_or(0),
            })
        })
        .collect()
}

/// A transcript's title: Claude's AI title, else the first typed prompt.
/// The newest `/rename` in a transcript after byte `from`: see `claude_transcript_marks`.
pub fn claude_renamed_title(path: &Path, from: u64) -> (u64, Option<String>) {
    let (offset, marks) = claude_transcript_marks(path, from);
    (offset, marks.title)
}

/// The newest `/rename` and `/color` in a transcript after byte `from`. Claude appends
/// `{"type":"custom-title","customTitle":…}` and `{"type":"agent-color","agentColor":…}`
/// lines for them. Also returns the offset just past the last complete line read, to
/// continue from: reading incrementally keeps long transcripts cheap to follow.
pub fn claude_transcript_marks(path: &Path, from: u64) -> (u64, TranscriptMarks) {
    use std::io::{BufRead, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return (from, TranscriptMarks::default());
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    // A shorter file than before was replaced: start over.
    let mut offset = if len < from { 0 } else { from };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return (from, TranscriptMarks::default());
    }
    let mut reader = BufReader::new(file);
    let mut marks = TranscriptMarks::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                // A partly written last line is read again next time.
                if line.last() != Some(&b'\n') {
                    break;
                }
                offset += n as u64;
                let has = |needle: &[u8]| line.windows(needle.len()).any(|w| w == needle);
                if !has(b"custom-title") && !has(b"agent-color") {
                    continue;
                }
                let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                let text = |key: &str| {
                    v.get(key)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(String::from)
                };
                match v.get("type").and_then(Value::as_str) {
                    Some("custom-title") => {
                        if let Some(t) = text("customTitle") {
                            marks.title = Some(t);
                        }
                    }
                    Some("agent-color") => {
                        if let Some(c) = text("agentColor") {
                            marks.color = Some(c);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    (offset, marks)
}

/// The sub-agents of a Claude Code session (its Agent/Task tool and teammates). Claude keeps
/// them next to the session's transcript, in `<session id>/subagents/`: `agent-<id>.jsonl` is
/// the sub-agent's own transcript and `agent-<id>.meta.json` describes it (`name`,
/// `description`, `agentType`, `model`). Verified against Claude Code 2.1.296 to 2.1.323.
pub fn claude_subagents(transcript: &Path, now: Millis) -> Vec<SubAgent> {
    let dir = transcript.with_extension("").join("subagents");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        let Some(id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("agent-"))
            .and_then(|n| n.strip_suffix(".jsonl"))
        else {
            continue;
        };
        let meta: Value = std::fs::read_to_string(dir.join(format!("agent-{id}.meta.json")))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        let text = |key: &str| {
            meta.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        };
        let last_activity_at = mtime_ms(&path);
        // Its last line being a finished assistant turn means it has reported back.
        let finished = last_json_line(&path).is_some_and(|v| {
            v.get("type").and_then(Value::as_str) == Some("assistant")
                && matches!(
                    v.get("message")
                        .and_then(|m| m.get("stop_reason"))
                        .and_then(Value::as_str),
                    Some("end_turn" | "stop_sequence" | "max_tokens" | "refusal")
                )
        });
        out.push(SubAgent {
            id: id.to_string(),
            name: text("name")
                .or_else(|| text("agentType"))
                .unwrap_or_else(|| "agent".into()),
            description: text("description"),
            model: text("model"),
            running: !finished && now - last_activity_at < SUBAGENT_QUIET_MS,
            started_at: first_timestamp(&path),
            last_activity_at,
        });
    }
    out
}

/// The `timestamp` of a transcript's first line.
fn first_timestamp(path: &Path) -> Option<Millis> {
    let file = std::fs::File::open(path).ok()?;
    let mut first = String::new();
    BufReader::new(file.take(DISCOVERY_READ_BYTES))
        .read_line(&mut first)
        .ok()?;
    let v: Value = serde_json::from_str(&first).ok()?;
    parse_rfc3339_ms(v.get("timestamp")?.as_str()?)
}

pub fn claude_transcript_title(path: &Path) -> Option<String> {
    read_transcript_head(path).map(|(title, _)| title)
}

/// Title and cwd from the start of a transcript.
fn read_transcript_head(path: &Path) -> Option<(String, Option<PathBuf>)> {
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file.take(DISCOVERY_READ_BYTES));
    let mut ai_title = None;
    let mut first_prompt = None;
    let mut cwd = None;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if cwd.is_none() {
            cwd = v.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        }
        match v.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                ai_title = v.get("aiTitle").and_then(Value::as_str).map(str::to_string);
            }
            Some("user") if first_prompt.is_none() => {
                first_prompt = user_prompt_text(&v);
            }
            _ => {}
        }
    }
    let title = ai_title.or(first_prompt)?;
    Some((one_line(&title, 100), cwd))
}

/// The text of a typed user prompt (not tool results, commands or meta messages).
pub(crate) fn user_prompt_text(v: &Value) -> Option<String> {
    if v.get("isMeta").and_then(Value::as_bool) == Some(true)
        || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))?
            .get("text")?
            .as_str()?
            .to_string(),
        _ => return None,
    };
    let t = text.trim();
    if t.is_empty() || t.starts_with('<') {
        return None;
    }
    Some(t.to_string())
}

impl AgentAdapter for ClaudeAdapter {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }

    fn binary_name(&self) -> &'static str {
        "claude"
    }

    fn api_key_vars(&self) -> &'static [&'static str] {
        &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]
    }

    /// `claude auth status --json` reports `authMethod: "claude.ai"` for a Pro/Max login.
    fn login_billing(&self, path: &Path, env: &UserEnv) -> Billing {
        let Some(out) = run_capture(path, &["auth", "status", "--json"], env) else {
            return Billing::Unknown;
        };
        let start = out.find('{').unwrap_or(0);
        match serde_json::from_str::<Value>(&out[start..])
            .ok()
            .and_then(|v| {
                v.get("authMethod")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .as_deref()
        {
            Some("claude.ai") => Billing::Subscription,
            Some("api_key" | "apiKey" | "console") => Billing::ApiKey,
            _ => Billing::Unknown,
        }
    }

    fn model_suggestions(&self, _env: &UserEnv) -> Vec<String> {
        // Aliases documented in `claude --help`; they track the latest models.
        ["opus", "sonnet", "haiku", "fable"]
            .map(String::from)
            .to_vec()
    }

    fn can_resume(&self) -> bool {
        true
    }

    fn can_fork(&self) -> bool {
        true
    }

    fn hooks(&self) -> bool {
        true
    }

    fn launch(&self, req: &LaunchRequest) -> CommandSpec {
        // `--session-id <uuid>` fixes Claude's own session id up front, so resume works
        // even before any hook has reported it. We reuse Termlab's id.
        let agent_session_id = req.session_id.to_string();
        let mut args = vec!["--session-id".into(), agent_session_id.clone()];
        Self::common_args(req, &mut args);
        Self::push_prompt(req, &mut args);
        CommandSpec::new(req, args, Some(agent_session_id))
    }

    fn resume(&self, agent_session_id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        let mut args = vec!["--resume".into(), agent_session_id.into()];
        Self::common_args(req, &mut args);
        Self::push_prompt(req, &mut args);
        Some(CommandSpec::new(req, args, Some(agent_session_id.into())))
    }

    /// `--resume <id> --fork-session` continues under a new id; the SessionStart hook
    /// reports it.
    fn fork(&self, agent_session_id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        let mut args = vec![
            "--resume".into(),
            agent_session_id.into(),
            "--fork-session".into(),
        ];
        Self::common_args(req, &mut args);
        Self::push_prompt(req, &mut args);
        Some(CommandSpec::new(req, args, None))
    }

    fn discover(&self, folder: &Path, env: &UserEnv) -> Vec<PastSession> {
        let Some(dir) = claude_project_dir(folder, env) else {
            return Vec::new();
        };
        sessions_from_files(jsonl_files(&dir), Some(folder), DISCOVERY_LIMIT)
    }

    /// Every project directory under `~/.claude/projects`, newest transcripts first. The
    /// folder comes from the transcript's own `cwd` (the directory name encoding is lossy).
    fn discover_all(&self, env: &UserEnv) -> Vec<PastSession> {
        let Some(root) = claude_config_dir(env).map(|c| c.join("projects")) else {
            return Vec::new();
        };
        let mut files: Vec<(i64, PathBuf)> = std::fs::read_dir(&root)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .flat_map(|e| jsonl_files(&e.path()))
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by(|a, b| b.0.cmp(&a.0));
        sessions_from_files(files, None, DISCOVER_ALL_LIMIT)
    }

    fn history_title(
        &self,
        _id: &str,
        transcript: Option<&Path>,
        _env: &UserEnv,
    ) -> Option<String> {
        let path = transcript?;
        claude_renamed_title(path, 0)
            .1
            .or_else(|| claude_transcript_title(path))
    }

    fn transcript_marks(&self, transcript: &Path, from: u64) -> (u64, TranscriptMarks) {
        claude_transcript_marks(transcript, from)
    }

    fn subagent_timeline(
        &self,
        _id: &str,
        subagent_id: &str,
        transcript: Option<&Path>,
        _env: &UserEnv,
    ) -> Vec<crate::transcript::TimelineEntry> {
        // The id becomes part of a file name: only what Claude's own ids contain.
        let safe = !subagent_id.is_empty()
            && subagent_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        match transcript {
            Some(t) if safe => crate::transcript::claude_subagent_timeline(
                &t.with_extension("")
                    .join("subagents")
                    .join(format!("agent-{subagent_id}.jsonl")),
            ),
            _ => Vec::new(),
        }
    }

    fn plan_usage_poll(&self) -> Option<PlanPoll> {
        Some(PlanPoll {
            every_ms: 5 * 60 * 1000,
            only_while_live: true,
        })
    }

    fn plan_usage(&self, program: &Path, env: &UserEnv, scratch: &Path) -> Option<PlanReport> {
        claude_fetch_usage(program, env, scratch)
    }

    fn subagents(
        &self,
        _id: &str,
        transcript: Option<&Path>,
        _since: Millis,
        now: Millis,
        _env: &UserEnv,
    ) -> Vec<SubAgent> {
        transcript
            .map(|t| claude_subagents(t, now))
            .unwrap_or_default()
    }

    /// `claude agents --json` lists the sessions running now, interactive and background.
    fn running(&self, program: &Path, env: &UserEnv) -> Vec<RunningSession> {
        run_capture(program, &["agents", "--json"], env)
            .map(|out| parse_agents_json(&out))
            .unwrap_or_default()
    }

    /// `claude attach <id>` opens a background session in this terminal; the session keeps
    /// running when the terminal detaches.
    fn attach(&self, running: &RunningSession, req: &LaunchRequest) -> Option<CommandSpec> {
        if running.kind != RunningKind::Background {
            return None;
        }
        let target = running
            .short_id
            .clone()
            .unwrap_or_else(|| running.agent_session_id.clone());
        Some(CommandSpec::new(
            req,
            vec!["attach".into(), target],
            Some(running.agent_session_id.clone()),
        ))
    }

    fn status_hints(&self) -> StatusHints {
        StatusHints {
            needs_input: vec!["do you want to proceed?", "do you want to make this edit"],
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_subagents_next_to_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("sess-1.jsonl");
        std::fs::write(&transcript, "{}\n").unwrap();
        let sub = dir.path().join("sess-1").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let user = r#"{"type":"user","isSidechain":true,"timestamp":"2026-10-13T13:47:01.446Z","message":{"role":"user","content":"go"}}"#;
        let tool = r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"tool_use","content":[]}}"#;
        let done = r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn","content":[]}}"#;
        // Still working: its last line is a tool call.
        std::fs::write(
            sub.join("agent-abuild-1.jsonl"),
            format!("{user}\n{tool}\n"),
        )
        .unwrap();
        std::fs::write(
            sub.join("agent-abuild-1.meta.json"),
            r#"{"agentType":"cp-build","description":"CP Vite build replacing Mix","name":"cp-build","model":"opus"}"#,
        )
        .unwrap();
        // Finished, and with no meta file.
        std::fs::write(
            sub.join("agent-aold-2.jsonl"),
            format!("{user}\n{tool}\n{done}\n"),
        )
        .unwrap();
        // Not a transcript.
        std::fs::write(sub.join("notes.txt"), "x").unwrap();

        let now = mtime_ms(&sub.join("agent-abuild-1.jsonl")) + 1_000;
        let mut got = claude_subagents(&transcript, now);
        got.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "abuild-1");
        assert_eq!(got[0].name, "cp-build");
        assert_eq!(
            got[0].description.as_deref(),
            Some("CP Vite build replacing Mix")
        );
        assert_eq!(got[0].model.as_deref(), Some("opus"));
        assert!(got[0].running);
        assert_eq!(
            got[0].started_at,
            parse_rfc3339_ms("2026-10-13T13:47:01.446Z")
        );
        assert_eq!(got[1].id, "aold-2");
        assert_eq!(got[1].name, "agent");
        assert!(!got[1].running);

        // Quiet for a long time without finishing: no longer counted as working.
        let later = claude_subagents(&transcript, now + SUBAGENT_QUIET_MS + 60_000);
        assert!(later.iter().all(|a| !a.running));
        // A session without sub-agents.
        assert!(claude_subagents(&dir.path().join("other.jsonl"), now).is_empty());
    }

    #[test]
    fn follows_renames_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let user =
            r#"{"type":"user","message":{"role":"user","content":"hi custom-title in text"}}"#;
        let rename =
            |t: &str| format!(r#"{{"type":"custom-title","customTitle":"{t}","sessionId":"s"}}"#);
        std::fs::write(&path, format!("{user}\n{}\n{user}\n", rename("First name"))).unwrap();

        let (offset, title) = claude_renamed_title(&path, 0);
        assert_eq!(title.as_deref(), Some("First name"));
        assert_eq!(offset, std::fs::metadata(&path).unwrap().len());

        // Nothing new: no title.
        assert_eq!(claude_renamed_title(&path, offset), (offset, None));

        // A later rename, plus a half-written line that's read again once complete.
        let more = format!("{}\n{}", rename("Second name"), &rename("Third")[..20]);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut f, more.as_bytes()).unwrap();
        let (offset2, title) = claude_renamed_title(&path, offset);
        assert_eq!(title.as_deref(), Some("Second name"));
        std::io::Write::write_all(&mut f, format!("{}\n", &rename("Third")[20..]).as_bytes())
            .unwrap();
        assert_eq!(
            claude_renamed_title(&path, offset2).1.as_deref(),
            Some("Third")
        );

        // The newest rename wins when reading from the start.
        assert_eq!(claude_renamed_title(&path, 0).1.as_deref(), Some("Third"));
    }

    use super::*;
    use crate::adapter::HookSetup;
    use crate::model::SessionId;

    fn req() -> LaunchRequest {
        LaunchRequest {
            session_id: SessionId::new(),
            program: PathBuf::from("/Users/me/.local/bin/claude"),
            cwd: PathBuf::from("/src/my app"),
            model: Some("opus".into()),
            permission: PermissionPreset::AutoEdit,
            first_message: Some("fix the tests".into()),
            hooks: None,
        }
    }

    #[test]
    fn launch_args() {
        let r = req();
        let spec = ClaudeAdapter.launch(&r);
        let id = r.session_id.to_string();
        assert_eq!(
            spec.args,
            vec![
                "--session-id",
                &id,
                "--model",
                "opus",
                "--permission-mode",
                "acceptEdits",
                "fix the tests"
            ]
        );
        assert_eq!(spec.agent_session_id.as_deref(), Some(id.as_str()));
        assert_eq!(
            spec.env,
            vec![("TERMLAB_SESSION_ID".to_string(), id.clone())]
        );
        assert_eq!(
            spec.display(),
            format!(
                "cd '/src/my app' && claude --session-id {id} --model opus --permission-mode acceptEdits 'fix the tests'"
            )
        );
    }

    #[test]
    fn launch_with_hooks_passes_a_settings_file() {
        let mut r = req();
        r.hooks = Some(HookSetup {
            binary: "/x/termlab-hook".into(),
            claude_settings: "/data/hooks/claude-settings.json".into(),
        });
        let spec = ClaudeAdapter.launch(&r);
        let i = spec.args.iter().position(|a| a == "--settings").unwrap();
        assert_eq!(spec.args[i + 1], "/data/hooks/claude-settings.json");
    }

    #[test]
    fn reads_the_usage_report_with_per_model_weeks() {
        // The answer Claude Code 2.1.323 gave to `get_usage` (trimmed to the parts used).
        let answer = json!({"type":"control_response","response":{"subtype":"success","request_id":"u","response":{
        "subscription_type":"max","rate_limits_available":true,
        "rate_limits":{"five_hour":null,"seven_day":{"utilization":64,"resets_at":"2026-11-14T13:59:59.768082+00:00"},
          "limits":[
            {"kind":"weekly_all","group":"weekly","percent":64,"resets_at":"2026-11-14T13:59:59.768082+00:00","scope":null,"is_active":true},
            {"kind":"weekly_scoped","group":"weekly","percent":25,"resets_at":"2026-11-14T13:59:59.768251+00:00",
             "scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":false}
          ]}}}});
        let w = claude_usage_windows(&answer).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].id.as_str(), w[0].used_percentage),
            ("seven_day", 64.0)
        );
        assert_eq!(w[0].label, None);
        assert_eq!(w[0].resets_at, parse_rfc3339_ms("2026-11-14T13:59:59.768Z"));
        assert_eq!(
            (w[1].id.as_str(), w[1].used_percentage),
            ("seven_day:Fable", 25.0)
        );
        assert_eq!(w[1].label.as_deref(), Some("Current week (Fable)"));

        // Without limits[]: the fixed windows.
        let old = json!({"response":{"response":{"rate_limits_available":true,
            "rate_limits":{"five_hour":{"utilization":12,"resets_at":"2026-11-13T15:00:00Z"},"seven_day":null}}}});
        let w = claude_usage_windows(&old).unwrap();
        assert_eq!(
            (w[0].id.as_str(), w[0].used_percentage),
            ("five_hour", 12.0)
        );
        // Plan limits don't apply (API key): an empty list, not a failure.
        let none =
            json!({"response":{"response":{"rate_limits_available":false,"rate_limits":null}}});
        assert_eq!(claude_usage_windows(&none), Some(Vec::new()));
        // Not a usage report at all.
        assert_eq!(
            claude_usage_windows(&json!({"response":{"subtype":"error"}})),
            None
        );
    }

    #[test]
    fn usage_report_carries_the_plan_and_says_when_limits_dont_apply() {
        let answer = json!({"response":{"response":{"subscription_type":"max","rate_limits_available":true,
            "rate_limits":{"limits":[{"kind":"weekly_all","group":"weekly","percent":64,"resets_at":"2026-11-14T13:59:59Z"}]}}}});
        let report = claude_usage_report(&answer).unwrap();
        assert_eq!(report.plan.as_deref(), Some("max"));
        assert_eq!(report.connected, Some(true));
        assert_eq!(report.windows.len(), 1);
        assert_eq!(report.note, None);
        let api_key =
            json!({"response":{"response":{"rate_limits_available":false,"rate_limits":null}}});
        let report = claude_usage_report(&api_key).unwrap();
        assert!(report.windows.is_empty());
        assert!(report.note.unwrap().contains("don't apply"));
    }

    #[test]
    fn reads_plan_limits_from_the_status_line_input() {
        // The example from Claude Code's status line documentation.
        let input = json!({
            "context_window": { "used_percentage": 8 },
            "rate_limits": {
                "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600 },
                "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600 },
                "spend_limit": { "used_percentage": 62.8, "resets_at": 1740787200, "used_usd": 314.12 },
                "odd": "ignored"
            }
        });
        let w = claude_plan_windows(&input);
        let ids: Vec<_> = w.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["five_hour", "seven_day", "spend_limit"]);
        assert_eq!(w[0].used_percentage, 23.5);
        assert_eq!(w[0].resets_at, Some(1_738_425_600_000));
        assert!(claude_plan_windows(&json!({ "context_window": {} })).is_empty());
    }

    #[test]
    fn status_line_keeps_the_users_own() {
        let hook = Path::new("/Apps/Term lab/termlab-hook");
        // None of their own: the hook draws a usage summary.
        let v = claude_settings_json(hook, None);
        assert_eq!(v["statusLine"]["type"], "command");
        assert_eq!(
            v["statusLine"]["command"],
            "'/Apps/Term lab/termlab-hook' claude StatusLine"
        );
        // Their own: the hook listens quietly and theirs draws the line.
        let v = claude_settings_json(hook, Some("~/.claude/statusline.sh"));
        let cmd = v["statusLine"]["command"].as_str().unwrap();
        assert!(cmd.contains("claude StatusLineQuiet"));
        assert!(cmd.contains("| { ~/.claude/statusline.sh\n}"));

        let home = tempfile::tempdir().unwrap();
        let env = UserEnv::from_vars([("HOME", home.path().to_str().unwrap())]);
        assert_eq!(claude_user_status_line(&env), None);
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"statusLine":{"type":"command","command":" jq -r .model.display_name "}}"#,
        )
        .unwrap();
        assert_eq!(
            claude_user_status_line(&env).as_deref(),
            Some("jq -r .model.display_name")
        );
    }

    #[test]
    fn settings_json_registers_each_event() {
        let v = claude_settings_json(Path::new("/Apps/Term lab/termlab-hook"), None);
        let hooks = v["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), HOOK_EVENTS.len());
        assert_eq!(
            v["hooks"]["Stop"][0]["hooks"][0]["command"],
            "'/Apps/Term lab/termlab-hook' claude Stop"
        );
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "*");
        assert!(v["hooks"]["Stop"][0].get("matcher").is_none());
    }

    #[test]
    fn launch_without_optional_parts() {
        let mut r = req();
        r.model = None;
        r.first_message = Some("  ".into());
        r.permission = PermissionPreset::AskFirst;
        let spec = ClaudeAdapter.launch(&r);
        assert_eq!(&spec.args[2..], ["--permission-mode", "manual"]);
    }

    #[test]
    fn prompt_starting_with_dash_is_not_a_flag() {
        let mut r = req();
        r.first_message = Some("-v explain".into());
        let spec = ClaudeAdapter.launch(&r);
        assert_eq!(&spec.args[spec.args.len() - 2..], ["--", "-v explain"]);
    }

    #[test]
    fn resume_and_fork_args() {
        let mut r = req();
        r.first_message = None;
        let spec = ClaudeAdapter.resume("abc-123", &r).unwrap();
        assert_eq!(
            spec.args,
            [
                "--resume",
                "abc-123",
                "--model",
                "opus",
                "--permission-mode",
                "acceptEdits"
            ]
        );
        assert_eq!(spec.agent_session_id.as_deref(), Some("abc-123"));
        let fork = ClaudeAdapter.fork("abc-123", &r).unwrap();
        assert_eq!(&fork.args[..3], ["--resume", "abc-123", "--fork-session"]);
        assert_eq!(fork.agent_session_id, None);
    }

    #[test]
    fn parses_claude_agents_json_and_attaches_to_background_sessions() {
        // Shape of `claude agents --json` from Claude Code 2.1.322.
        let out = r#"[
          {"id":"2749a623","cwd":"/w/site","kind":"background","startedAt":1,"sessionId":"2749a623-dd14-48bd-aa18-7c1d22600c2c","name":"Microsoft integration","state":"blocked"},
          {"pid":23767,"id":"bbbb7fdb","cwd":"/w/app","kind":"background","startedAt":2,"sessionId":"bbbb7fdb-eef3","name":"app","status":"busy","state":"working"},
          {"pid":21898,"id":"e5f6a7b8","cwd":"/w/cli","kind":"interactive","startedAt":3,"sessionId":"e5f6a7b8-dc95","name":"cli","status":"idle","state":"done"}
        ]"#;
        let r = parse_agents_json(out);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].status, Some(SessionStatus::NeedsYou));
        assert_eq!(r[1].status, Some(SessionStatus::Running));
        assert_eq!(r[2].kind, RunningKind::Interactive);
        assert_eq!(r[2].status, Some(SessionStatus::Idle));

        let mut q = req();
        q.cwd = "/w/app".into();
        let spec = ClaudeAdapter.attach(&r[1], &q).unwrap();
        assert_eq!(spec.args, ["attach", "bbbb7fdb"]);
        assert_eq!(spec.agent_session_id.as_deref(), Some("bbbb7fdb-eef3"));
        // A session open in another terminal window can't be attached to.
        assert!(ClaudeAdapter.attach(&r[2], &q).is_none());
        assert!(parse_agents_json("not json").is_empty());
    }

    #[test]
    fn path_encoding_matches_claude() {
        assert_eq!(
            encode_claude_path(Path::new("/Users/me/Documents/session manager")),
            "-Users-me-Documents-session-manager"
        );
        assert_eq!(encode_claude_path(Path::new("/a/b.c_d")), "-a-b-c-d");
    }

    #[test]
    fn discovers_transcripts() {
        let home = tempfile::tempdir().unwrap();
        let folder = Path::new("/work/my api");
        let dir = home
            .path()
            .join(".claude/projects")
            .join(encode_claude_path(folder));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("11111111-1111-1111-1111-111111111111.jsonl"),
            concat!(
                r#"{"type":"user","message":{"role":"user","content":"<command-name>/model</command-name>"},"cwd":"/work/my api"}"#, "\n",
                r#"{"type":"user","message":{"role":"user","content":"Add rate limiting\nto the API"},"cwd":"/work/my api"}"#, "\n",
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("22222222-2222-2222-2222-222222222222.jsonl"),
            concat!(
                r#"{"type":"ai-title","aiTitle":"Retry queue worker"}"#, "\n",
                r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"move retries"}]},"cwd":"/work/my api"}"#, "\n",
            ),
        )
        .unwrap();
        // No user prompt at all: not offered.
        std::fs::write(
            dir.join("33333333-3333-3333-3333-333333333333.jsonl"),
            "{}\n",
        )
        .unwrap();

        // Another project, found only by the everywhere scan.
        let other = home.path().join(".claude/projects/-elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("44444444-4444-4444-4444-444444444444.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":"tidy the docs"},"cwd":"/elsewhere"}"#,
        )
        .unwrap();

        let env = UserEnv::from_vars([("HOME", home.path().to_str().unwrap())]);
        let all = ClaudeAdapter.discover_all(&env);
        assert_eq!(all.len(), 3);
        assert!(
            all.iter()
                .any(|s| s.cwd == Path::new("/elsewhere") && s.title == "tidy the docs")
        );
        let mut found = ClaudeAdapter.discover(folder, &env);
        found.sort_by(|a, b| a.agent_session_id.cmp(&b.agent_session_id));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "Add rate limiting to the API");
        assert_eq!(found[1].title, "Retry queue worker");
        assert_eq!(found[1].cwd, PathBuf::from("/work/my api"));
    }
}
