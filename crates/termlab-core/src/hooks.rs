//! Hook events from agents, delivered by `termlab-hook` over a Unix socket.
//!
//! Wire format (one message per connection): a JSON header line, then the agent's raw
//! payload (usually JSON):
//!
//! ```text
//! {"v":1,"session":"<termlab session id>","agent":"claude","event":"PreToolUse"}\n
//! {"session_id":"…","transcript_path":"…","tool_name":"Bash",…}
//! ```

use serde::Deserialize;
use serde_json::Value;

use crate::adapter::one_line;
use crate::model::{AgentId, SessionId, SessionStatus};

/// Messages larger than this are truncated (tool inputs can be big; we only need fields).
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct HookMessage {
    pub session: SessionId,
    pub agent: AgentId,
    pub event: String,
    pub payload: Value,
}

#[derive(Deserialize)]
struct Header {
    session: String,
    agent: String,
    event: String,
}

pub fn parse_message(bytes: &[u8]) -> Option<HookMessage> {
    let text = String::from_utf8_lossy(bytes);
    let (header, payload) = text.split_once('\n').unwrap_or((&text, ""));
    let h: Header = serde_json::from_str(header).ok()?;
    Some(HookMessage {
        session: h.session.parse().ok()?,
        agent: h.agent.parse().ok()?,
        event: h.event,
        payload: serde_json::from_str(payload.trim()).unwrap_or(Value::Null),
    })
}

/// What a hook event means for its session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HookEffect {
    pub status: Option<SessionStatus>,
    /// `Some(None)` clears the pending approval detail.
    pub approval_detail: Option<Option<String>>,
    pub agent_session_id: Option<String>,
    pub transcript_path: Option<String>,
    /// One line for the Timeline tab; `None` to not record the event.
    pub summary: Option<String>,
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// A short description of a tool call: the command, file or pattern it acts on.
pub fn describe_tool(tool: &str, input: &Value) -> String {
    let arg = [
        "command",
        "cmd",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
        "prompt",
    ]
    .iter()
    .find_map(|k| s(input, k));
    match (tool, arg) {
        ("Bash" | "shell" | "exec_command" | "local_shell", Some(cmd)) => {
            format!("$ {}", one_line(cmd, 160))
        }
        (t, Some(a)) => format!("{t} {}", one_line(a, 160)),
        (t, None) => t.to_string(),
    }
}

pub fn interpret(msg: &HookMessage) -> HookEffect {
    match msg.agent {
        AgentId::Claude => interpret_claude(&msg.event, &msg.payload),
        AgentId::Codex => interpret_codex(&msg.event, &msg.payload),
        _ => HookEffect::default(),
    }
}

fn interpret_claude(event: &str, p: &Value) -> HookEffect {
    let mut e = HookEffect {
        agent_session_id: s(p, "session_id").map(String::from),
        transcript_path: s(p, "transcript_path").map(String::from),
        ..Default::default()
    };
    let tool = || {
        describe_tool(
            s(p, "tool_name").unwrap_or("tool"),
            p.get("tool_input").unwrap_or(&Value::Null),
        )
    };
    match event {
        "SessionStart" => {
            e.status = Some(SessionStatus::Idle);
            e.approval_detail = Some(None);
            e.summary = Some(format!(
                "Session started ({})",
                s(p, "source").unwrap_or("startup")
            ));
        }
        "UserPromptSubmit" => {
            e.status = Some(SessionStatus::Running);
            e.approval_detail = Some(None);
            e.summary = s(p, "prompt").map(|t| format!("You: {}", one_line(t, 200)));
        }
        "PreToolUse" => {
            e.status = Some(SessionStatus::Running);
            e.summary = Some(tool());
        }
        "PostToolUse" => {
            // The tool ran, so any permission prompt for it has been answered.
            e.status = Some(SessionStatus::Running);
            e.approval_detail = Some(None);
        }
        "PermissionRequest" => {
            e.status = Some(SessionStatus::NeedsYou);
            let t = tool();
            e.approval_detail = Some(Some(t.clone()));
            e.summary = Some(format!("Asked permission: {t}"));
        }
        "Notification" => {
            let message = s(p, "message").unwrap_or_default();
            let kind = s(p, "notification_type").unwrap_or_default();
            if kind == "permission_prompt" || message.to_lowercase().contains("permission") {
                e.status = Some(SessionStatus::NeedsYou);
            } else if kind == "idle_prompt" {
                e.status = Some(SessionStatus::Idle);
            }
            if !message.is_empty() {
                e.summary = Some(one_line(message, 200));
            }
        }
        "Stop" => {
            e.status = Some(SessionStatus::Idle);
            e.approval_detail = Some(None);
            e.summary = Some(match s(p, "last_assistant_message") {
                Some(m) if !m.trim().is_empty() => format!("Finished: {}", one_line(m, 200)),
                _ => "Finished its turn".into(),
            });
        }
        "SessionEnd" => {
            e.summary = Some(format!(
                "Session ended ({})",
                s(p, "reason").unwrap_or("exit")
            ));
        }
        _ => {}
    }
    e
}

/// Codex's `notify` program gets `{"type":"agent-turn-complete","thread-id":…,…}`.
fn interpret_codex(event: &str, p: &Value) -> HookEffect {
    if event != "notify" || s(p, "type") != Some("agent-turn-complete") {
        return HookEffect::default();
    }
    HookEffect {
        status: Some(SessionStatus::Idle),
        approval_detail: Some(None),
        agent_session_id: s(p, "thread-id").map(String::from),
        transcript_path: None,
        summary: Some(match s(p, "last-assistant-message") {
            Some(m) if !m.trim().is_empty() => format!("Finished: {}", one_line(m, 200)),
            _ => "Finished its turn".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(agent: &str, event: &str, payload: Value) -> Vec<u8> {
        let id = SessionId::new();
        format!(
            "{{\"v\":1,\"session\":\"{id}\",\"agent\":\"{agent}\",\"event\":\"{event}\"}}\n{payload}"
        )
        .into_bytes()
    }

    #[test]
    fn parses_header_and_payload() {
        let m = parse_message(&msg("claude", "Stop", json!({"session_id": "abc"}))).unwrap();
        assert_eq!(m.agent, AgentId::Claude);
        assert_eq!(m.event, "Stop");
        assert_eq!(m.payload["session_id"], "abc");
        // Empty or broken payloads still parse; a broken header doesn't.
        assert!(
            parse_message(b"{\"v\":1,\"session\":\"x\",\"agent\":\"claude\",\"event\":\"Stop\"}\n")
                .is_none()
        );
        let m = parse_message(&msg("codex", "notify", Value::Null)).unwrap();
        assert_eq!(m.payload, Value::Null);
        assert!(parse_message(b"garbage").is_none());
    }

    #[test]
    fn claude_permission_request() {
        let p = json!({
            "session_id": "s1", "transcript_path": "/t.jsonl", "hook_event_name": "PermissionRequest",
            "tool_name": "Bash", "tool_input": {"command": "psql -f migrate.sql"}
        });
        let e = interpret_claude("PermissionRequest", &p);
        assert_eq!(e.status, Some(SessionStatus::NeedsYou));
        assert_eq!(
            e.approval_detail,
            Some(Some("$ psql -f migrate.sql".into()))
        );
        assert_eq!(e.agent_session_id.as_deref(), Some("s1"));
        assert_eq!(e.transcript_path.as_deref(), Some("/t.jsonl"));
    }

    #[test]
    fn claude_notifications() {
        let perm = json!({"message": "Claude needs your permission to use Bash", "notification_type": "permission_prompt"});
        assert_eq!(
            interpret_claude("Notification", &perm).status,
            Some(SessionStatus::NeedsYou)
        );
        let idle = json!({"message": "Claude is waiting for your input", "notification_type": "idle_prompt"});
        assert_eq!(
            interpret_claude("Notification", &idle).status,
            Some(SessionStatus::Idle)
        );
        let other = json!({"message": "Signed in", "notification_type": "auth_success"});
        assert_eq!(interpret_claude("Notification", &other).status, None);
    }

    #[test]
    fn codex_notify() {
        let p = json!({"type": "agent-turn-complete", "thread-id": "019a", "last-assistant-message": "Done."});
        let e = interpret_codex("notify", &p);
        assert_eq!(e.status, Some(SessionStatus::Idle));
        assert_eq!(e.agent_session_id.as_deref(), Some("019a"));
        assert_eq!(
            interpret_codex("notify", &json!({"type": "other"})),
            HookEffect::default()
        );
    }

    #[test]
    fn describes_tools() {
        assert_eq!(
            describe_tool("Edit", &json!({"file_path": "src/a.rs"})),
            "Edit src/a.rs"
        );
        assert_eq!(describe_tool("Task", &json!({})), "Task");
    }
}
