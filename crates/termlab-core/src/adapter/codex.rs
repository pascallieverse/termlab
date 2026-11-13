//! Codex CLI (`codex`). Verified against codex-cli 0.167.0.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    AgentAdapter, CommandSpec, LaunchRequest, PastSession, PlanPoll, PlanReport, SUBAGENT_QUIET_MS,
    StatusHints, home, last_json_line, mtime_ms, one_line, run_capture, same_dir, toml_string,
};
use crate::env::UserEnv;
use crate::model::{AgentId, Billing, Millis, PermissionPreset, SubAgent, UsageWindow};

pub struct CodexAdapter;

const DISCOVERY_LIMIT: usize = 40;
const DISCOVER_ALL_LIMIT: usize = 400;
/// Rollout files to look at when discovering (newest first).
const SCAN_LIMIT: usize = 3000;

impl CodexAdapter {
    /// 0.167.0 has no `--full-auto`; `-a` takes only `on-request` and `never`.
    fn permission_args(p: PermissionPreset) -> [&'static str; 4] {
        match p {
            PermissionPreset::AskFirst => ["-s", "read-only", "-a", "on-request"],
            PermissionPreset::AutoEdit => ["-s", "workspace-write", "-a", "on-request"],
            PermissionPreset::FullAuto => ["-s", "workspace-write", "-a", "never"],
        }
    }

    fn options(req: &LaunchRequest) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(model) = req.model() {
            args.extend(["-m".into(), model.into()]);
        }
        args.extend(Self::permission_args(req.permission).map(String::from));
        if let Some(h) = &req.hooks {
            // `notify` runs a program with a JSON argument when a turn completes. Setting it
            // with `-c` overrides it for this run only; ~/.codex/config.toml is untouched.
            args.extend([
                "-c".into(),
                format!(
                    "notify=[{}, \"codex\", \"notify\"]",
                    toml_string(&h.binary.to_string_lossy())
                ),
            ]);
        }
        args
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

fn codex_home(env: &UserEnv) -> Option<PathBuf> {
    env.get("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home(env).map(|h| h.join(".codex")))
}

/// Rollout files, newest first: `sessions/YYYY/MM/DD/rollout-<ts>-<id>.jsonl`.
fn rollout_files(env: &UserEnv) -> Vec<PathBuf> {
    let Some(root) = codex_home(env).map(|h| h.join("sessions")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let sorted_dirs = |p: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default();
        v.sort();
        v.reverse();
        v
    };
    'outer: for year in sorted_dirs(&root) {
        for month in sorted_dirs(&year) {
            for day in sorted_dirs(&month) {
                for f in sorted_dirs(&day) {
                    if f.extension().is_some_and(|x| x == "jsonl") {
                        out.push(f);
                        if out.len() >= SCAN_LIMIT {
                            break 'outer;
                        }
                    }
                }
            }
        }
    }
    out
}

struct Meta {
    id: String,
    cwd: PathBuf,
    timestamp: Option<String>,
    subagent: bool,
    /// For a sub-agent's rollout: who spawned it and what Codex calls it.
    parent: Option<String>,
    nickname: Option<String>,
    role: Option<String>,
    agent_path: Option<String>,
}

fn read_meta(path: &Path) -> Option<Meta> {
    let file = std::fs::File::open(path).ok()?;
    // The first line can hold long base instructions; cap it.
    let mut first = String::new();
    BufReader::new(file.take(4 * 1024 * 1024))
        .read_line(&mut first)
        .ok()?;
    let v: Value = serde_json::from_str(&first).ok()?;
    if v.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = v.get("payload")?;
    // `source: {"subagent": {"thread_spawn": {"parent_thread_id", "agent_nickname", …}}}`
    let spawn = p
        .get("source")
        .and_then(|s| s.get("subagent"))
        .and_then(|s| s.get("thread_spawn"));
    let spawn_text = |key: &str| {
        spawn
            .and_then(|s| s.get(key))
            .or_else(|| p.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(String::from)
    };
    Some(Meta {
        parent: spawn_text("parent_thread_id"),
        nickname: spawn_text("agent_nickname"),
        role: spawn_text("agent_role"),
        agent_path: spawn_text("agent_path"),
        id: p.get("id")?.as_str()?.to_string(),
        cwd: PathBuf::from(p.get("cwd")?.as_str()?),
        timestamp: p.get("timestamp").and_then(Value::as_str).map(String::from),
        subagent: p.get("thread_source").and_then(Value::as_str) == Some("subagent")
            || p.get("source").and_then(|s| s.get("subagent")).is_some(),
    })
}

/// `session_index.jsonl` maps thread ids to their names.
fn thread_names(env: &UserEnv) -> HashMap<String, String> {
    let Some(path) = codex_home(env).map(|h| h.join("session_index.jsonl")) else {
        return HashMap::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            Some((
                v.get("id")?.as_str()?.to_string(),
                v.get("thread_name")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn first_user_message(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    for line in BufReader::new(file.take(2 * 1024 * 1024))
        .lines()
        .map_while(Result::ok)
        .skip(1)
    {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let p = v.get("payload");
        if v.get("type").and_then(Value::as_str) == Some("event_msg")
            && p.and_then(|p| p.get("type")).and_then(Value::as_str) == Some("user_message")
        {
            return p?.get("message")?.as_str().map(String::from);
        }
    }
    None
}

/// Top-level Codex sessions (subagents skipped), newest first, optionally in one folder.
fn codex_sessions(env: &UserEnv, folder: Option<&Path>, limit: usize) -> Vec<PastSession> {
    let names = thread_names(env);
    rollout_files(env)
        .into_iter()
        .filter_map(|p| read_meta(&p).map(|m| (m, p)))
        .filter(|(m, _)| !m.subagent && folder.is_none_or(|f| same_dir(&m.cwd, f)))
        .filter_map(|(m, path)| {
            let title = names
                .get(&m.id)
                .cloned()
                .or_else(|| first_user_message(&path))?;
            Some(PastSession {
                agent: AgentId::Codex,
                agent_session_id: m.id,
                title: one_line(&title, 100),
                cwd: m.cwd,
                last_activity_at: mtime_ms(&path),
                path,
            })
        })
        .take(limit)
        .collect()
}

/// The rollout file for a Codex session id.
pub fn codex_rollout_path(env: &UserEnv, id: &str) -> Option<PathBuf> {
    let suffix = format!("-{id}.jsonl");
    rollout_files(env)
        .into_iter()
        .find(|p| p.to_string_lossy().ends_with(&suffix))
}

/// Codex's plan limits, from its newest rollout that has any: every `token_count` event
/// carries `rate_limits: {"primary": {"used_percent", "window_minutes", "resets_at"},
/// "secondary": …}` (resets in Unix seconds). Returns the windows and the event's time.
/// Verified against codex-cli 0.169.
pub fn codex_plan_usage(env: &UserEnv) -> Option<(Vec<UsageWindow>, Millis)> {
    let mut recent: Vec<(Millis, PathBuf)> = rollout_files(env)
        .into_iter()
        .take(60)
        .map(|p| (mtime_ms(&p), p))
        .collect();
    recent.sort_by(|a, b| b.0.cmp(&a.0));
    recent
        .into_iter()
        .take(10)
        .find_map(|(mtime, path)| rollout_rate_limits(&path, mtime))
}

/// The windows in one of Codex's rate-limit objects: `primary` and `secondary`, each with a
/// percentage, a window length in minutes (300 = 5 hours, 10080 = a week) and a reset time
/// in Unix seconds. Rollouts spell the fields `used_percent` / `window_minutes` /
/// `resets_at`; the app server spells them `usedPercent` / `windowDurationMins` / `resetsAt`.
/// `scope` names a limit other than the account's main one.
fn rate_limit_windows(limits: &Value, scope: Option<&str>) -> Vec<UsageWindow> {
    let field = |w: &Value, names: [&str; 2]| names.iter().find_map(|n| w.get(n).cloned());
    ["primary", "secondary"]
        .iter()
        .filter_map(|slot| {
            let w = limits.get(slot).filter(|w| w.is_object())?;
            let minutes =
                field(w, ["window_minutes", "windowDurationMins"]).and_then(|v| v.as_i64());
            let (id, label) = match minutes {
                Some(300) => ("five_hour".to_string(), None),
                Some(10_080) => ("seven_day".to_string(), None),
                Some(m) if m % 1_440 == 0 => (
                    format!("{slot}_{m}m"),
                    Some(format!("{}-day limit", m / 1_440)),
                ),
                Some(m) => (
                    format!("{slot}_{m}m"),
                    Some(format!("{}-hour limit", (m + 30) / 60)),
                ),
                None => ((*slot).to_string(), Some(format!("{slot} limit"))),
            };
            let (id, label) = match scope {
                Some(scope) => {
                    let window = match id.as_str() {
                        "five_hour" => "5 hours".to_string(),
                        "seven_day" => "Current week".to_string(),
                        _ => label.unwrap_or_else(|| (*slot).to_string()),
                    };
                    (format!("{id}:{scope}"), Some(format!("{window} ({scope})")))
                }
                None => (id, label),
            };
            Some(UsageWindow {
                id,
                label,
                used_percentage: field(w, ["used_percent", "usedPercent"])?.as_f64()?,
                resets_at: field(w, ["resets_at", "resetsAt"])
                    .and_then(|v| v.as_i64())
                    .map(|s| s * 1000),
            })
        })
        .collect()
}

/// The windows in the app server's answer to `account/rateLimits/read`: the account's main
/// limit, then any other limits it lists (by their name).
pub fn codex_rate_limits_answer(result: &Value) -> Vec<UsageWindow> {
    let Some(main) = result.get("rateLimits").filter(|v| v.is_object()) else {
        return Vec::new();
    };
    let main_id = main.get("limitId").and_then(Value::as_str);
    let mut windows = rate_limit_windows(main, None);
    for (id, limit) in result
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if Some(id.as_str()) == main_id {
            continue;
        }
        let name = limit
            .get("limitName")
            .and_then(Value::as_str)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(id);
        windows.extend(rate_limit_windows(limit, Some(name)));
    }
    windows
}

/// Asks Codex for its limits right now: starts `codex app-server` (JSON-RPC over stdio),
/// initialises, and sends `account/rateLimits/read`. No thread is started and no prompt is
/// sent, so it uses none of the plan. Verified against codex-cli 0.169.
fn codex_fetch_usage(program: &Path, env: &UserEnv, scratch: &Path) -> Option<PlanReport> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new(program)
        .arg("app-server")
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
        let stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        // The reply to request `id`, skipping the notifications in between.
        let reply = |id: i64| loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let v = rx.recv_timeout(left).ok()?;
            if v.get("id").and_then(Value::as_i64) == Some(id) {
                return Some(v);
            }
        };
        stdin
            .write_all(b"{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"termlab\",\"title\":\"Termlab\",\"version\":\"0.1.0\"}}}\n")
            .ok()?;
        stdin.flush().ok()?;
        reply(1)?;
        stdin
            .write_all(b"{\"method\":\"initialized\"}\n{\"id\":2,\"method\":\"account/rateLimits/read\"}\n")
            .ok()?;
        stdin.flush().ok()?;
        // Closing its input (dropping `stdin`) ends the server.
        reply(2)
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
    let answer = answer?;
    let result = answer.get("result")?;
    Some(PlanReport {
        windows: codex_rate_limits_answer(result),
        at: crate::model::now_ms(),
        connected: Some(true),
        plan: result
            .get("rateLimits")
            .and_then(|r| r.get("planType"))
            .and_then(Value::as_str)
            .filter(|p| !p.trim().is_empty())
            .map(String::from),
        note: None,
    })
}

fn rollout_rate_limits(path: &Path, mtime: Millis) -> Option<(Vec<UsageWindow>, Millis)> {
    use std::io::{Seek, SeekFrom};
    const TAIL: u64 = 512 * 1024;
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    // Bytes, not a String: the tail may start inside a multi-byte character.
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let event: Value = text
        .lines()
        .rev()
        .filter(|l| l.contains("\"rate_limits\""))
        .find_map(|l| serde_json::from_str(l).ok())?;
    let payload = event.get("payload")?;
    let limits = payload
        .get("rate_limits")
        .or_else(|| payload.get("info")?.get("rate_limits"))?;
    let windows = rate_limit_windows(limits, None);
    let at = event
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_ms)
        .unwrap_or(mtime);
    (!windows.is_empty()).then_some((windows, at))
}

/// The agents a Codex session spawned. Each has its own rollout whose `session_meta` names
/// the parent thread; a rollout ending in `task_complete` has reported back. Verified
/// against codex-cli 0.167 to 0.169.
pub fn codex_subagents(
    env: &UserEnv,
    parent_id: &str,
    since: Millis,
    now: Millis,
) -> Vec<SubAgent> {
    rollout_files(env)
        .into_iter()
        .take(400)
        .filter(|p| mtime_ms(p) >= since - 5_000)
        .filter_map(|p| read_meta(&p).map(|m| (m, p)))
        .filter(|(m, _)| m.parent.as_deref() == Some(parent_id))
        .map(|(m, path)| {
            let last_activity_at = mtime_ms(&path);
            let finished = last_json_line(&path).is_some_and(|v| {
                v.get("type").and_then(Value::as_str) == Some("event_msg")
                    && matches!(
                        v.get("payload")
                            .and_then(|p| p.get("type"))
                            .and_then(Value::as_str),
                        Some("task_complete" | "turn_aborted")
                    )
            });
            // "/root/ai_project_analysis_ui" → "ai project analysis ui"
            let task = m
                .agent_path
                .as_deref()
                .and_then(|p| p.rsplit('/').next())
                .filter(|s| !s.is_empty())
                .map(|s| s.replace('_', " "));
            SubAgent {
                name: m.nickname.unwrap_or_else(|| "agent".into()),
                description: m.role.or(task),
                model: None,
                running: !finished && now - last_activity_at < SUBAGENT_QUIET_MS,
                started_at: m.timestamp.as_deref().and_then(parse_rfc3339_ms),
                last_activity_at,
                id: m.id,
            }
        })
        .collect()
}

/// Codex picks its own session id, so Termlab finds it afterwards: the newest top-level
/// rollout in `folder` written since `since` that no other session has claimed.
pub fn codex_find_new_rollout(
    env: &UserEnv,
    folder: &Path,
    since: Millis,
    claimed: &HashSet<String>,
) -> Option<(String, PathBuf)> {
    rollout_files(env)
        .into_iter()
        .take(200)
        .filter(|p| mtime_ms(p) >= since - 5_000)
        .filter_map(|p| read_meta(&p).map(|m| (m, p)))
        .filter(|(m, _)| !m.subagent && !claimed.contains(&m.id) && same_dir(&m.cwd, folder))
        .filter(|(m, _)| {
            // Session start must not predate ours (allowing for clock rounding).
            m.timestamp
                .as_deref()
                .and_then(parse_rfc3339_ms)
                .is_none_or(|t| t >= since - 5_000)
        })
        .map(|(m, p)| (m.id, p))
        .next()
}

/// Minimal RFC 3339 (`2026-11-06T10:14:28.641Z`, `…T13:59:59.768082+00:00`) to Unix
/// milliseconds.
pub(crate) fn parse_rfc3339_ms(s: &str) -> Option<Millis> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    // A numeric UTC offset (`+02:00`, `-0500`): the time is that far ahead of UTC.
    let (time, offset_ms) = match time.rfind(['+', '-']) {
        Some(i) => {
            let digits: String = time[i + 1..].chars().filter(char::is_ascii_digit).collect();
            let hours: i64 = digits.get(0..2)?.parse().ok()?;
            let minutes: i64 = digits.get(2..4).unwrap_or("0").parse().ok()?;
            let sign = if time.as_bytes()[i] == b'-' { -1 } else { 1 };
            (&time[..i], sign * (hours * 60 + minutes) * 60_000)
        }
        None => (time, 0),
    };
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = hms.split(':').map(|x| x.parse::<i64>().ok());
    let (h, mi, se) = (t.next()??, t.next()??, t.next()??);
    let ms: i64 = format!("{:0<3}", &frac.chars().take(3).collect::<String>())
        .parse()
        .ok()?;
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3_600 + mi * 60 + se) * 1_000) + ms - offset_ms)
}

impl AgentAdapter for CodexAdapter {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn binary_name(&self) -> &'static str {
        "codex"
    }

    fn api_key_vars(&self) -> &'static [&'static str] {
        &["OPENAI_API_KEY", "CODEX_API_KEY"]
    }

    /// `codex login status` prints "Logged in using ChatGPT" for a subscription login.
    fn login_billing(&self, path: &Path, env: &UserEnv) -> Billing {
        match run_capture(path, &["login", "status"], env) {
            Some(o) if o.contains("ChatGPT") => Billing::Subscription,
            Some(o) if o.to_lowercase().contains("api key") => Billing::ApiKey,
            _ => Billing::Unknown,
        }
    }

    /// Listed models from Codex's own cache (`~/.codex/models_cache.json`).
    fn model_suggestions(&self, env: &UserEnv) -> Vec<String> {
        let Some(path) = codex_home(env).map(|h| h.join("models_cache.json")) else {
            return Vec::new();
        };
        let Some(v) = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            return Vec::new();
        };
        v.get("models")
            .and_then(Value::as_array)
            .map(|ms| {
                ms.iter()
                    .filter(|m| m.get("visibility").and_then(Value::as_str) != Some("hide"))
                    .filter_map(|m| m.get("slug").and_then(Value::as_str).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
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
        let mut args = Self::options(req);
        Self::push_prompt(req, &mut args);
        CommandSpec::new(req, args, None)
    }

    /// `codex resume <SESSION_ID> [PROMPT]` takes the same options.
    fn resume(&self, agent_session_id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        let mut args = vec!["resume".into()];
        args.extend(Self::options(req));
        args.push(agent_session_id.into());
        Self::push_prompt(req, &mut args);
        Some(CommandSpec::new(req, args, Some(agent_session_id.into())))
    }

    /// `codex fork <SESSION_ID> [PROMPT]`; the fork gets a new id, found from its rollout.
    fn fork(&self, agent_session_id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        let mut args = vec!["fork".into()];
        args.extend(Self::options(req));
        args.push(agent_session_id.into());
        Self::push_prompt(req, &mut args);
        Some(CommandSpec::new(req, args, None))
    }

    fn discover(&self, folder: &Path, env: &UserEnv) -> Vec<PastSession> {
        codex_sessions(env, Some(folder), DISCOVERY_LIMIT)
    }

    fn discover_all(&self, env: &UserEnv) -> Vec<PastSession> {
        codex_sessions(env, None, DISCOVER_ALL_LIMIT)
    }

    fn find_started_session(
        &self,
        folder: &Path,
        since: Millis,
        claimed: &HashSet<String>,
        env: &UserEnv,
    ) -> Option<(String, Option<PathBuf>)> {
        codex_find_new_rollout(env, folder, since, claimed).map(|(id, path)| (id, Some(path)))
    }

    fn subagent_timeline(
        &self,
        id: &str,
        subagent_id: &str,
        _transcript: Option<&Path>,
        env: &UserEnv,
    ) -> Vec<crate::transcript::TimelineEntry> {
        // Only a rollout that names this session as its parent.
        codex_rollout_path(env, subagent_id)
            .filter(|p| read_meta(p).is_some_and(|m| m.parent.as_deref() == Some(id)))
            .map(|p| crate::transcript::codex_timeline(&p))
            .unwrap_or_default()
    }

    fn plan_usage_poll(&self) -> Option<PlanPoll> {
        Some(PlanPoll {
            every_ms: 2 * 60 * 1000,
            only_while_live: true,
        })
    }

    /// Asks Codex itself; if that fails, falls back to what its newest session file recorded.
    fn plan_usage(&self, program: &Path, env: &UserEnv, scratch: &Path) -> Option<PlanReport> {
        codex_fetch_usage(program, env, scratch)
            .filter(|r| !r.windows.is_empty())
            .or_else(|| {
                codex_plan_usage(env).map(|(windows, at)| PlanReport {
                    windows,
                    at,
                    ..PlanReport::default()
                })
            })
    }

    fn subagents(
        &self,
        id: &str,
        _transcript: Option<&Path>,
        since: Millis,
        now: Millis,
        env: &UserEnv,
    ) -> Vec<SubAgent> {
        codex_subagents(env, id, since, now)
    }

    /// Codex's thread name, else the first prompt in the rollout.
    fn history_title(&self, id: &str, transcript: Option<&Path>, env: &UserEnv) -> Option<String> {
        thread_names(env)
            .remove(id)
            .or_else(|| crate::transcript::codex_progress(transcript?).goal)
    }

    fn status_hints(&self) -> StatusHints {
        StatusHints {
            needs_input: vec![
                "would you like to run the following command?",
                "would you like to make the following edits?",
                "allow command?",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::HookSetup;
    use crate::model::SessionId;

    fn req() -> LaunchRequest {
        LaunchRequest {
            session_id: SessionId::new(),
            program: "/bin/codex".into(),
            cwd: "/src/api".into(),
            model: Some("gpt-6-sol".into()),
            permission: PermissionPreset::AskFirst,
            first_message: Some("add idempotency keys".into()),
            hooks: Some(HookSetup {
                binary: "/Apps/Term lab/termlab-hook".into(),
                claude_settings: "/unused".into(),
            }),
        }
    }

    #[test]
    fn launch_args() {
        let spec = CodexAdapter.launch(&req());
        assert_eq!(
            spec.args,
            [
                "-m",
                "gpt-6-sol",
                "-s",
                "read-only",
                "-a",
                "on-request",
                "-c",
                r#"notify=["/Apps/Term lab/termlab-hook", "codex", "notify"]"#,
                "add idempotency keys"
            ]
        );
        assert_eq!(spec.agent_session_id, None);
    }

    #[test]
    fn presets() {
        let mut r = req();
        r.hooks = None;
        r.first_message = None;
        r.model = None;
        r.permission = PermissionPreset::FullAuto;
        assert_eq!(
            CodexAdapter.launch(&r).args,
            ["-s", "workspace-write", "-a", "never"]
        );
        r.permission = PermissionPreset::AutoEdit;
        assert_eq!(
            CodexAdapter.launch(&r).args,
            ["-s", "workspace-write", "-a", "on-request"]
        );
    }

    #[test]
    fn resume_and_fork_put_the_id_after_options() {
        let mut r = req();
        r.hooks = None;
        r.first_message = None;
        r.model = None;
        let spec = CodexAdapter.resume("019a-e71", &r).unwrap();
        assert_eq!(
            spec.args,
            ["resume", "-s", "read-only", "-a", "on-request", "019a-e71"]
        );
        let fork = CodexAdapter.fork("019a-e71", &r).unwrap();
        assert_eq!(fork.args[0], "fork");
        assert_eq!(fork.args.last().unwrap(), "019a-e71");
    }

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:01.5Z"), Some(1_500));
        assert_eq!(
            parse_rfc3339_ms("2026-11-06T10:14:28.641Z"),
            Some(1_793_960_068_641)
        );
    }

    fn write_rollout(home: &Path, day: &str, id: &str, cwd: &str, extra: &str, subagent: bool) {
        let dir = home.join(".codex/sessions").join(day);
        std::fs::create_dir_all(&dir).unwrap();
        let src = if subagent {
            r#","thread_source":"subagent""#
        } else {
            ""
        };
        std::fs::write(
            dir.join(format!("rollout-2026-11-06T10-00-00-{id}.jsonl")),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\",\"timestamp\":\"2026-11-06T10:00:00.000Z\"{src}}}}}\n{extra}"
            ),
        )
        .unwrap();
    }

    #[test]
    fn reads_plan_limits_from_the_newest_rollout() {
        let home = tempfile::tempdir().unwrap();
        // Shaped like codex-cli 0.169's token_count events.
        let event = |pct: u32| {
            format!(
                "{{\"timestamp\":\"2026-11-13T10:56:55.916Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":null,\"rate_limits\":{{\"limit_id\":\"codex\",\"primary\":{{\"used_percent\":{pct}.0,\"window_minutes\":10080,\"resets_at\":1794572183}},\"secondary\":null,\"plan_type\":\"prolite\"}}}}}}\n"
            )
        };
        write_rollout(
            home.path(),
            "2026/11/13",
            "aaa",
            "/work/api",
            &format!("{}{}", event(40), event(44)),
            false,
        );
        let env = UserEnv::from_vars([("HOME", home.path().to_str().unwrap())]);
        let (windows, at) = codex_plan_usage(&env).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "seven_day");
        // The last event in the file wins.
        assert_eq!(windows[0].used_percentage, 44.0);
        assert_eq!(windows[0].resets_at, Some(1_794_572_183_000));
        assert_eq!(Some(at), parse_rfc3339_ms("2026-11-13T10:56:55.916Z"));

        let empty = tempfile::tempdir().unwrap();
        let env = UserEnv::from_vars([("HOME", empty.path().to_str().unwrap())]);
        assert!(codex_plan_usage(&env).is_none());
    }

    #[test]
    fn reads_the_app_servers_rate_limits() {
        // The answer codex-cli 0.169 gave to `account/rateLimits/read` (trimmed).
        let result: Value = serde_json::from_str(
            r#"{"ordinaryUsageAllowed":true,
                "rateLimits":{"limitId":"codex","limitName":null,
                  "primary":{"usedPercent":0,"windowDurationMins":10080,"resetsAt":1795176999},
                  "secondary":null,"planType":"prolite"},
                "rateLimitsByLimitId":{
                  "codex":{"limitId":"codex","primary":{"usedPercent":0,"windowDurationMins":10080,"resetsAt":1795176999},"secondary":null},
                  "codex_other":{"limitId":"codex_other","limitName":"Spark",
                    "primary":{"usedPercent":12.5,"windowDurationMins":300,"resetsAt":1795000000},"secondary":null}}}"#,
        )
        .unwrap();
        let w = codex_rate_limits_answer(&result);
        let got: Vec<_> = w
            .iter()
            .map(|w| (w.id.as_str(), w.label.as_deref(), w.used_percentage))
            .collect();
        assert_eq!(
            got,
            [
                ("seven_day", None, 0.0),
                ("five_hour:Spark", Some("5 hours (Spark)"), 12.5)
            ]
        );
        assert_eq!(w[0].resets_at, Some(1_795_176_999_000));
        assert!(codex_rate_limits_answer(&serde_json::json!({"rateLimits": null})).is_empty());
    }

    #[test]
    fn parses_timestamps_with_offsets() {
        let utc = parse_rfc3339_ms("2026-11-14T13:59:59.768Z").unwrap();
        assert_eq!(
            parse_rfc3339_ms("2026-11-14T13:59:59.768082+00:00"),
            Some(utc)
        );
        assert_eq!(parse_rfc3339_ms("2026-11-14T15:59:59.768+02:00"), Some(utc));
        assert_eq!(parse_rfc3339_ms("2026-11-14T08:59:59.768-0500"), Some(utc));
    }

    #[test]
    fn finds_the_agents_a_session_spawned() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".codex/sessions/2026/11/06");
        std::fs::create_dir_all(&dir).unwrap();
        // Shaped like codex-cli 0.169's sub-agent rollouts.
        let child = |id: &str, parent: &str, nick: &str, tail: &str| {
            std::fs::write(
                dir.join(format!("rollout-2026-11-06T15-14-53-{id}.jsonl")),
                format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/work/api\",\"timestamp\":\"2026-11-06T15:14:53.000Z\",\"thread_source\":\"subagent\",\"source\":{{\"subagent\":{{\"thread_spawn\":{{\"parent_thread_id\":\"{parent}\",\"depth\":1,\"agent_path\":\"/root/ai_project_analysis_ui\",\"agent_nickname\":\"{nick}\",\"agent_role\":null}}}}}}}}}}\n{tail}"
                ),
            )
            .unwrap();
        };
        child(
            "kid-1",
            "parent-1",
            "McClintock",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\"}}\n",
        );
        child(
            "kid-2",
            "parent-1",
            "Noether",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n",
        );
        child("kid-3", "someone-else", "Curie", "");
        write_rollout(
            home.path(),
            "2026/11/06",
            "parent-1",
            "/work/api",
            "",
            false,
        );

        let env = UserEnv::from_vars([("HOME", home.path().to_str().unwrap())]);
        let now = mtime_ms(&dir.join("rollout-2026-11-06T15-14-53-kid-1.jsonl")) + 1_000;
        let mut got = codex_subagents(&env, "parent-1", 0, now);
        got.sort_by(|a, b| a.id.cmp(&b.id));
        let summary: Vec<_> = got
            .iter()
            .map(|a| (a.id.as_str(), a.name.as_str(), a.running))
            .collect();
        assert_eq!(
            summary,
            [("kid-1", "McClintock", true), ("kid-2", "Noether", false)]
        );
        assert_eq!(
            got[0].description.as_deref(),
            Some("ai project analysis ui")
        );
        assert_eq!(
            got[0].started_at,
            parse_rfc3339_ms("2026-11-06T15:14:53.000Z")
        );
        // Rollouts older than the session aren't looked at.
        assert!(codex_subagents(&env, "parent-1", now + 3_600_000, now).is_empty());
    }

    #[test]
    fn discovers_rollouts_for_a_folder() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        write_rollout(h, "2026/11/06", "aaa", "/work/api", "", false);
        write_rollout(
            h,
            "2026/11/05",
            "bbb",
            "/work/api",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"split the state\"}}\n",
            false,
        );
        write_rollout(h, "2026/11/06", "ccc", "/work/other", "", false);
        write_rollout(h, "2026/11/06", "ddd", "/work/api", "", true);
        std::fs::write(
            h.join(".codex/session_index.jsonl"),
            "{\"id\":\"aaa\",\"thread_name\":\"Idempotency keys\"}\n",
        )
        .unwrap();

        let env = UserEnv::from_vars([("HOME", h.to_str().unwrap())]);
        let found = CodexAdapter.discover(Path::new("/work/api"), &env);
        let got: Vec<_> = found
            .iter()
            .map(|s| (s.agent_session_id.as_str(), s.title.as_str()))
            .collect();
        assert_eq!(
            got,
            [("aaa", "Idempotency keys"), ("bbb", "split the state")]
        );
        assert!(codex_rollout_path(&env, "bbb").is_some());

        let claimed: HashSet<String> = ["aaa".to_string()].into();
        let (id, _) = codex_find_new_rollout(&env, Path::new("/work/api"), 0, &claimed).unwrap();
        assert_eq!(id, "bbb");
    }
}
