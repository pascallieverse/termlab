//! `termlab-hook <agent> <event> [payload-json]`
//!
//! Run by agents' hooks (Claude Code hooks, Codex `notify`). Forwards the event to the
//! Termlab daemon over the Unix socket named by `TERMLAB_HOOK_SOCKET`, tagged with
//! `TERMLAB_SESSION_ID`. Both variables are set only on sessions Termlab started, so outside
//! Termlab this does nothing.
//!
//! Rules: never print anything for a hook (Claude Code feeds some hooks' stdout back to the
//! model), always exit 0, and finish quickly even if Termlab isn't running.
//!
//! The one exception is `StatusLine`: Termlab registers this binary as Claude Code's status
//! line to receive the plan's rate-limit usage, and what a status line prints is what Claude
//! Code shows, so it prints a short usage summary (`5h 23% · week 63%`). `StatusLineQuiet`
//! forwards the same data and prints nothing (used when chaining to the user's own status
//! line).

use std::io::{IsTerminal, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

/// Payloads are capped; Termlab only needs a few fields.
const MAX_PAYLOAD: u64 = 1024 * 1024;
const STDIN_TIMEOUT: Duration = Duration::from_secs(2);
const SOCKET_TIMEOUT: Duration = Duration::from_secs(1);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(agent), Some(event)) = (args.get(1), args.get(2)) else {
        return;
    };
    // Codex's `notify` passes the payload as the last argument; Claude Code uses stdin.
    let payload = match args.get(3) {
        Some(arg) => arg.clone().into_bytes(),
        None => read_stdin(),
    };
    let _ = forward(agent, event, &payload);
    if event == "StatusLine"
        && let Some(line) = status_line(&String::from_utf8_lossy(&payload))
    {
        println!("{line}");
    }
    // Exit 0 no matter what happened.
}

fn forward(agent: &str, event: &str, payload: &[u8]) -> Option<()> {
    let session = std::env::var("TERMLAB_SESSION_ID").ok()?;
    let socket = std::env::var("TERMLAB_HOOK_SOCKET").ok()?;
    let header = format!(
        "{{\"v\":1,\"session\":{},\"agent\":{},\"event\":{}}}\n",
        json_string(&session),
        json_string(agent),
        json_string(event)
    );
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_write_timeout(Some(SOCKET_TIMEOUT)).ok()?;
    stream.write_all(header.as_bytes()).ok()?;
    stream.write_all(payload).ok()?;
    stream.flush().ok()
}

/// A one-line summary of the plan limits in Claude Code's status-line input
/// (`rate_limits.five_hour` / `.seven_day`, each with `used_percentage`), or None when it has
/// none (API-key billing, or before the session's first response).
fn status_line(json: &str) -> Option<String> {
    let limits = &json[json.find("\"rate_limits\"")?..];
    let parts: Vec<String> = [("\"five_hour\"", "5h"), ("\"seven_day\"", "week")]
        .into_iter()
        .filter_map(|(key, label)| Some(format!("{label} {}%", used_percentage(limits, key)?)))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// The rounded `used_percentage` inside the object that follows `key`.
fn used_percentage(json: &str, key: &str) -> Option<u32> {
    const FIELD: &str = "\"used_percentage\"";
    let rest = &json[json.find(key)? + key.len()..];
    let object = &rest[..rest.find('}')?];
    let value = object[object.find(FIELD)? + FIELD.len()..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start();
    let number: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    number.parse::<f64>().ok().map(|n| n.round() as u32)
}

/// Reads stdin with a time limit, so a hook whose stdin is never closed can't hang.
fn read_stdin() -> Vec<u8> {
    if std::io::stdin().is_terminal() {
        return Vec::new();
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::stdin().take(MAX_PAYLOAD).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(STDIN_TIMEOUT).unwrap_or_default()
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
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
    fn summarises_plan_limits_for_the_status_line() {
        // Shaped like Claude Code's status-line input.
        let input = r#"{"context_window":{"used_percentage":8},"rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1738425600},"seven_day":{"used_percentage": 62.8,"resets_at":1738857600},"seven_day_opus":{"used_percentage":99}},"vim":{}}"#;
        assert_eq!(
            status_line(input).as_deref(),
            Some("5h 24% \u{b7} week 63%")
        );
        // One window only.
        let input = r#"{"rate_limits":{"seven_day":{"resets_at":1,"used_percentage":41}}}"#;
        assert_eq!(status_line(input).as_deref(), Some("week 41%"));
        // No plan limits (API-key billing, or before the first response).
        assert_eq!(
            status_line(r#"{"context_window":{"used_percentage":8}}"#),
            None
        );
        assert_eq!(status_line(r#"{"rate_limits":{}}"#), None);
        assert_eq!(status_line(""), None);
    }

    #[test]
    fn escapes_json() {
        assert_eq!(json_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(json_string("x\ny"), r#""x\u000ay""#);
    }
}
