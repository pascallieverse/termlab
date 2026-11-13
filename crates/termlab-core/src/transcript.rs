//! Reading agents' transcripts: token use, a timeline of what happened, and the
//! goal/progress summary used for hand-offs. Read-only; formats verified against real files
//! from Claude Code 2.1.x and codex-cli 0.167.0.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{one_line, parse_rfc3339_ms};
use crate::hooks::describe_tool;
use crate::model::{Millis, Usage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    Prompt,
    Tool,
    Message,
    Hook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    pub at: Millis,
    pub kind: TimelineKind,
    pub text: String,
}

/// What a session set out to do and where it got to, for hand-offs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub goal: Option<String>,
    pub last_message: Option<String>,
}

fn lines(path: &Path) -> impl Iterator<Item = Value> {
    std::fs::File::open(path)
        .ok()
        .into_iter()
        .flat_map(|f| BufReader::new(f).lines().map_while(Result::ok))
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
}

fn ts(v: &Value) -> Millis {
    v.get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_ms)
        .unwrap_or(0)
}

fn u(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

// ---- Claude Code ----------------------------------------------------------------

/// Per-million-token prices: (input, output, 5-min cache write, 1-hour cache write,
/// cache read). First-party Anthropic API list prices, from the Claude API reference
/// (cached 2026-11-03).
fn claude_prices(model: &str) -> Option<[f64; 5]> {
    let m = model.to_lowercase();
    let table: &[(&str, [f64; 5])] = &[
        ("claude-fable-5-2", [8.0, 40.0, 10.0, 16.0, 0.20]),
        ("claude-mythos-5-2", [8.0, 40.0, 10.0, 16.0, 0.20]),
        ("claude-fable-5-1", [10.0, 50.0, 12.5, 20.0, 0.25]),
        ("claude-mythos-5-1", [10.0, 50.0, 12.5, 20.0, 0.25]),
        ("claude-fable-5", [10.0, 50.0, 12.5, 20.0, 1.00]),
        ("claude-mythos-5", [10.0, 50.0, 12.5, 20.0, 1.00]),
        ("claude-opus-5-5", [4.0, 20.0, 5.0, 8.0, 0.20]),
        ("claude-opus-5", [5.0, 25.0, 6.25, 10.0, 0.50]),
        ("claude-opus-4-8", [5.0, 25.0, 6.25, 10.0, 0.50]),
        ("claude-opus-4-7", [5.0, 25.0, 6.25, 10.0, 0.50]),
        ("claude-opus-4-6", [5.0, 25.0, 6.25, 10.0, 0.50]),
        ("claude-sonnet-5-5", [2.0, 10.0, 2.5, 4.0, 0.10]),
        ("claude-sonnet-5", [2.0, 10.0, 2.5, 4.0, 0.20]),
        ("claude-sonnet-4-6", [3.0, 15.0, 3.75, 6.0, 0.30]),
        ("claude-haiku-5-5", [0.10, 0.50, 0.125, 0.20, 0.01]),
        ("claude-haiku-4-5", [1.0, 5.0, 1.25, 2.0, 0.10]),
    ];
    // Longest matching prefix, so "claude-opus-5-5" isn't priced as "claude-opus-5".
    table
        .iter()
        .filter(|(prefix, _)| m.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, p)| *p)
}

/// Current Claude models have a 1M-token context window; Haiku 4.5 has 200K.
fn claude_context_window(model: Option<&str>, context_tokens: u64) -> u64 {
    match model {
        Some(m) if m.contains("haiku-4") => 200_000,
        Some(m) if claude_prices(m).is_some() => 1_000_000,
        // Unknown model: assume the smaller window unless usage already exceeds it.
        _ if context_tokens > 200_000 => 1_000_000,
        _ => 200_000,
    }
}

#[derive(Default, Clone)]
struct ClaudeMsgUsage {
    input: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    cache_read: u64,
    output: u64,
    model: Option<String>,
    fast: bool,
}

impl ClaudeMsgUsage {
    fn from(msg: &Value) -> Option<Self> {
        let usage = msg.get("usage")?;
        let creation = u(usage, "cache_creation_input_tokens");
        let detail = usage.get("cache_creation");
        let (w5, w1) = match detail {
            Some(d) => (
                u(d, "ephemeral_5m_input_tokens"),
                u(d, "ephemeral_1h_input_tokens"),
            ),
            None => (creation, 0),
        };
        Some(Self {
            input: u(usage, "input_tokens"),
            cache_write_5m: w5,
            cache_write_1h: w1,
            cache_read: u(usage, "cache_read_input_tokens"),
            output: u(usage, "output_tokens"),
            model: msg.get("model").and_then(Value::as_str).map(String::from),
            fast: usage.get("speed").and_then(Value::as_str) == Some("fast"),
        })
    }

    fn cost(&self) -> Option<f64> {
        let p = claude_prices(self.model.as_deref()?)?;
        // Fast mode is 2x on Opus 5.5; other models' fast rates aren't tabled here.
        let mult = match (self.fast, self.model.as_deref()) {
            (false, _) => 1.0,
            (true, Some(m)) if m.starts_with("claude-opus-5-5") => 2.0,
            (true, _) => return None,
        };
        let per = |tokens: u64, price: f64| tokens as f64 / 1_000_000.0 * price * mult;
        Some(
            per(self.input, p[0])
                + per(self.output, p[1])
                + per(self.cache_write_5m, p[2])
                + per(self.cache_write_1h, p[3])
                + per(self.cache_read, p[4]),
        )
    }
}

/// Token use from a Claude Code transcript. Claude writes one line per content block,
/// each repeating the message's usage, so usage is de-duplicated by message id.
pub fn claude_usage(path: &Path, with_cost: bool) -> Option<Usage> {
    let mut by_id: HashMap<String, ClaudeMsgUsage> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for v in lines(path) {
        if v.get("type").and_then(Value::as_str) != Some("assistant")
            || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let Some(mu) = ClaudeMsgUsage::from(msg) else {
            continue;
        };
        let id = msg
            .get("id")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| format!("line-{}", order.len()));
        if !by_id.contains_key(&id) {
            order.push(id.clone());
        }
        by_id.insert(id, mu);
    }
    let last = by_id.get(order.last()?)?;
    let context =
        last.input + last.cache_write_5m + last.cache_write_1h + last.cache_read + last.output;
    let total = by_id
        .values()
        .map(|m| m.input + m.cache_write_5m + m.cache_write_1h + m.output)
        .sum();
    let cost_usd = if with_cost {
        by_id
            .values()
            .map(ClaudeMsgUsage::cost)
            .sum::<Option<f64>>()
    } else {
        None
    };
    Some(Usage {
        context_tokens: context,
        context_window: Some(claude_context_window(last.model.as_deref(), context)),
        total_tokens: total,
        cost_usd,
        model: last.model.clone(),
    })
}

fn claude_text_blocks(msg: &Value) -> Option<String> {
    let content = msg.get("content")?;
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let t: Vec<&str> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            (!t.is_empty()).then(|| t.join("\n"))
        }
        _ => None,
    }
}

pub fn claude_timeline(path: &Path) -> Vec<TimelineEntry> {
    claude_timeline_of(path, false)
}

/// The steps of a sub-agent, from its own transcript (`subagents/agent-<id>.jsonl`). Every
/// line there is marked `isSidechain`, which a session's own timeline leaves out.
pub fn claude_subagent_timeline(path: &Path) -> Vec<TimelineEntry> {
    claude_timeline_of(path, true)
}

fn claude_timeline_of(path: &Path, sidechain: bool) -> Vec<TimelineEntry> {
    let mut out = Vec::new();
    for mut v in lines(path) {
        let is_sidechain = v.get("isSidechain").and_then(Value::as_bool) == Some(true);
        if is_sidechain != sidechain {
            continue;
        }
        // The prompt reader skips sidechain lines; here they are the conversation.
        if let Some(obj) = v.as_object_mut() {
            obj.remove("isSidechain");
        }
        let at = ts(&v);
        match v.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let Some(t) = crate::adapter::claude_user_prompt(&v) {
                    out.push(TimelineEntry {
                        at,
                        kind: TimelineKind::Prompt,
                        text: one_line(&t, 300),
                    });
                }
            }
            Some("assistant") => {
                let Some(msg) = v.get("message") else {
                    continue;
                };
                if let Some(blocks) = msg.get("content").and_then(Value::as_array) {
                    for b in blocks {
                        match b.get("type").and_then(Value::as_str) {
                            Some("tool_use") => out.push(TimelineEntry {
                                at,
                                kind: TimelineKind::Tool,
                                text: describe_tool(
                                    b.get("name").and_then(Value::as_str).unwrap_or("tool"),
                                    b.get("input").unwrap_or(&Value::Null),
                                ),
                            }),
                            Some("text") => {
                                if let Some(t) = b.get("text").and_then(Value::as_str)
                                    && !t.trim().is_empty()
                                {
                                    out.push(TimelineEntry {
                                        at,
                                        kind: TimelineKind::Message,
                                        text: one_line(t, 300),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub fn claude_progress(path: &Path) -> Progress {
    let mut p = Progress::default();
    for v in lines(path) {
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        match v.get("type").and_then(Value::as_str) {
            Some("user") if p.goal.is_none() => p.goal = crate::adapter::claude_user_prompt(&v),
            Some("assistant") => {
                if let Some(t) = v.get("message").and_then(claude_text_blocks)
                    && !t.trim().is_empty()
                {
                    p.last_message = Some(t);
                }
            }
            _ => {}
        }
    }
    p
}

// ---- Codex ----------------------------------------------------------------------

/// Token use from the last `token_count` event in a Codex rollout.
pub fn codex_usage(path: &Path) -> Option<Usage> {
    let mut last: Option<Value> = None;
    let mut model = None;
    for v in lines(path) {
        let p = v.get("payload");
        match (
            v.get("type").and_then(Value::as_str),
            p.and_then(|p| p.get("type")).and_then(Value::as_str),
        ) {
            (Some("event_msg"), Some("token_count")) => {
                if let Some(info) = p.and_then(|p| p.get("info")).filter(|i| !i.is_null()) {
                    last = Some(info.clone());
                }
            }
            (Some("turn_context"), _) => {
                if let Some(m) = p.and_then(|p| p.get("model")).and_then(Value::as_str) {
                    model = Some(m.to_string());
                }
            }
            _ => {}
        }
    }
    let info = last?;
    let last_usage = info.get("last_token_usage")?;
    let total_usage = info.get("total_token_usage")?;
    Some(Usage {
        context_tokens: u(last_usage, "input_tokens") + u(last_usage, "output_tokens"),
        context_window: info.get("model_context_window").and_then(Value::as_u64),
        total_tokens: u(total_usage, "input_tokens")
            .saturating_sub(u(total_usage, "cached_input_tokens"))
            + u(total_usage, "output_tokens"),
        cost_usd: None,
        model,
    })
}

pub fn codex_timeline(path: &Path) -> Vec<TimelineEntry> {
    let mut out = Vec::new();
    for v in lines(path) {
        let at = ts(&v);
        let Some(p) = v.get("payload") else { continue };
        let kind = p.get("type").and_then(Value::as_str);
        match (v.get("type").and_then(Value::as_str), kind) {
            (Some("event_msg"), Some("user_message")) => {
                if let Some(m) = p.get("message").and_then(Value::as_str) {
                    out.push(TimelineEntry {
                        at,
                        kind: TimelineKind::Prompt,
                        text: one_line(m, 300),
                    });
                }
            }
            (Some("event_msg"), Some("agent_message")) => {
                if let Some(m) = p.get("message").and_then(Value::as_str) {
                    out.push(TimelineEntry {
                        at,
                        kind: TimelineKind::Message,
                        text: one_line(m, 300),
                    });
                }
            }
            (Some("response_item"), Some("function_call" | "custom_tool_call")) => {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("tool");
                let args = p
                    .get("arguments")
                    .or_else(|| p.get("input"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let parsed = serde_json::from_str::<Value>(args).unwrap_or(Value::Null);
                let text = if let Some(cmd) = codex_exec_command(args) {
                    format!("$ {}", one_line(&cmd, 160))
                } else if parsed.is_object() {
                    describe_tool(name, &parsed)
                } else {
                    format!("{name} {}", one_line(args, 160))
                };
                out.push(TimelineEntry {
                    at,
                    kind: TimelineKind::Tool,
                    text: text.trim().to_string(),
                });
            }
            _ => {}
        }
    }
    out
}

/// Codex's code-mode `exec` tool wraps shell commands as `tools.exec_command({cmd:"…"})`.
fn codex_exec_command(code: &str) -> Option<String> {
    let start = code.find("exec_command(")?;
    let rest = &code[start..];
    let key = rest.find("cmd:")?;
    let quoted = rest[key + 4..].trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
    None
}

pub fn codex_progress(path: &Path) -> Progress {
    let mut p = Progress::default();
    for e in codex_timeline(path) {
        match e.kind {
            TimelineKind::Prompt if p.goal.is_none() => p.goal = Some(e.text),
            TimelineKind::Message => p.last_message = Some(e.text),
            _ => {}
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(lines: &[&str]) -> tempfile::NamedTempFile {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), lines.join("\n")).unwrap();
        f
    }

    #[test]
    fn claude_usage_dedupes_blocks_and_prices_api_key_sessions() {
        let f = write(&[
            r#"{"type":"user","timestamp":"2026-11-12T10:00:00.000Z","message":{"role":"user","content":"Move retries to a queue"}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-12T10:00:05.000Z","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"text","text":"Reading the code."}],"usage":{"input_tokens":10,"cache_creation_input_tokens":1000000,"cache_read_input_tokens":0,"output_tokens":100,"cache_creation":{"ephemeral_5m_input_tokens":1000000,"ephemeral_1h_input_tokens":0}}}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-12T10:00:05.000Z","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}],"usage":{"input_tokens":10,"cache_creation_input_tokens":1000000,"cache_read_input_tokens":0,"output_tokens":100,"cache_creation":{"ephemeral_5m_input_tokens":1000000,"ephemeral_1h_input_tokens":0}}}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-12T10:01:00.000Z","message":{"id":"m2","model":"claude-opus-5-5","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":1000000,"output_tokens":1000000}}}"#,
        ]);
        let u = claude_usage(f.path(), true).unwrap();
        assert_eq!(u.context_tokens, 2_000_000);
        assert_eq!(u.context_window, Some(1_000_000));
        assert_eq!(u.total_tokens, 10 + 1_000_000 + 100 + 1_000_000);
        // 1M 5-min writes at $5 + 1M reads at $0.20 + ~1M output at $20 (+ tiny input).
        let cost = u.cost_usd.unwrap();
        assert!((cost - 25.2022).abs() < 0.001, "{cost}");
        assert_eq!(claude_usage(f.path(), false).unwrap().cost_usd, None);

        let tl = claude_timeline(f.path());
        let kinds: Vec<_> = tl.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                TimelineKind::Prompt,
                TimelineKind::Message,
                TimelineKind::Tool,
                TimelineKind::Message
            ]
        );
        assert_eq!(tl[2].text, "$ npm test");
        let p = claude_progress(f.path());
        assert_eq!(p.goal.as_deref(), Some("Move retries to a queue"));
        assert_eq!(p.last_message.as_deref(), Some("Done."));
    }

    #[test]
    fn prices_prefer_the_longest_model_prefix() {
        assert_eq!(claude_prices("claude-opus-5-5").unwrap()[0], 4.0);
        assert_eq!(claude_prices("claude-opus-5").unwrap()[0], 5.0);
        assert_eq!(claude_prices("claude-opus-5-20260101").unwrap()[0], 5.0);
        assert!(claude_prices("gpt-6").is_none());
        assert_eq!(claude_context_window(Some("claude-haiku-4-5"), 10), 200_000);
        assert_eq!(claude_context_window(None, 10), 200_000);
    }

    #[test]
    fn codex_usage_and_timeline() {
        let f = write(&[
            r#"{"type":"session_meta","payload":{"id":"x","cwd":"/w"}}"#,
            r#"{"type":"turn_context","payload":{"model":"gpt-6-sol"}}"#,
            r#"{"timestamp":"2026-11-06T10:00:00.000Z","type":"event_msg","payload":{"type":"user_message","message":"add idempotency"}}"#,
            r#"{"timestamp":"2026-11-06T10:00:01.000Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":\"cargo test\"}"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":21008,"cached_input_tokens":12032,"output_tokens":165,"total_tokens":21173},"last_token_usage":{"input_tokens":21008,"cached_input_tokens":12032,"output_tokens":165,"total_tokens":21173},"model_context_window":475000}}}"#,
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"Added the table."}}"#,
        ]);
        let u = codex_usage(f.path()).unwrap();
        assert_eq!(u.context_tokens, 21_173);
        assert_eq!(u.context_window, Some(475_000));
        assert_eq!(u.total_tokens, 21008 - 12032 + 165);
        assert_eq!(u.model.as_deref(), Some("gpt-6-sol"));
        assert_eq!(u.context_percent(), Some(4));
        assert_eq!(
            codex_exec_command(
                r#"text(await tools.exec_command({cmd:"touch probe.txt",max_output_tokens:1000}));"#
            )
            .as_deref(),
            Some("touch probe.txt")
        );
        let tl = codex_timeline(f.path());
        assert_eq!(tl.len(), 3);
        assert_eq!(tl[1].text, "$ cargo test");
        let p = codex_progress(f.path());
        assert_eq!(p.goal.as_deref(), Some("add idempotency"));
        assert_eq!(p.last_message.as_deref(), Some("Added the table."));
    }
}
