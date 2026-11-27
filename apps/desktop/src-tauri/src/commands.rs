//! Tauri commands. Most requests pass straight through to `termlabd`.

use std::path::PathBuf;

use serde_json::Value;
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use termlab_core::ipc::Request;
use termlab_core::model::SessionId;

use crate::AppState;
use crate::daemon::DaemonStatus;

type CmdResult<T> = Result<T, String>;

/// Sends one request to the daemon. `request` is `{ "method": …, "params": … }`.
#[tauri::command]
pub async fn daemon_call(state: State<'_, AppState>, request: Request) -> CmdResult<Value> {
    match request {
        Request::Attach { .. } | Request::Detach { .. } => {
            Err("use attach_session / detach_session".into())
        }
        req => state.daemon.call(req).await,
    }
}

/// Streams a session's output (scrollback first, then live) as raw bytes on `on_output`.
#[tauri::command]
pub async fn attach_session(
    state: State<'_, AppState>,
    id: SessionId,
    on_output: Channel<InvokeResponseBody>,
) -> CmdResult<Value> {
    state.daemon.attach(id, on_output).await
}

#[tauri::command]
pub async fn detach_session(state: State<'_, AppState>, id: SessionId) -> CmdResult<()> {
    state.daemon.detach(id).await
}

#[tauri::command]
pub fn daemon_status(state: State<'_, AppState>) -> DaemonStatus {
    state.daemon.status()
}

/// Restarts an outdated daemon now; its sessions are resumed by the new one.
#[tauri::command]
pub async fn restart_daemon(state: State<'_, AppState>) -> CmdResult<()> {
    state.daemon.restart().await
}

/// Opens the user's terminal app in `cwd` running `command` (macOS: a `.command` file
/// opened with the default terminal; Linux: `x-terminal-emulator`).
#[tauri::command]
pub fn open_in_terminal(
    state: State<'_, AppState>,
    cwd: PathBuf,
    command: String,
) -> CmdResult<()> {
    if !cwd.is_dir() {
        return Err(format!("{} is not a folder", cwd.display()));
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = state.data_dir.join("open-in-terminal");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join(format!("{}.command", uuid_like()));
        let script = format!("#!/bin/zsh -l\n{command}\nexec \"$SHELL\" -l\n");
        std::fs::write(&file, script).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        std::process::Command::new("open")
            .arg(&file)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(windows)]
    {
        let _ = &state;
        // `start` opens a new console window; `/k` keeps it open after the command.
        std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "start", "", "cmd.exe", "/k"])
            .arg(&command)
            .current_dir(&cwd)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("couldn't open a terminal: {e}"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = &state;
        let script = format!("{command}; exec \"${{SHELL:-sh}}\" -l");
        // The desktop's default terminal where the system names one, then common ones.
        // GNOME Terminal takes the command after `--`, the others after `-e`.
        const TERMINALS: &[(&str, &str)] = &[
            ("x-terminal-emulator", "-e"),
            ("gnome-terminal", "--"),
            ("konsole", "-e"),
            ("xfce4-terminal", "-x"),
            ("xterm", "-e"),
        ];
        for (terminal, flag) in TERMINALS {
            let started = std::process::Command::new(terminal)
                .arg(flag)
                .args(["sh", "-lc", &script])
                .current_dir(&cwd)
                .spawn();
            match started {
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("couldn't open a terminal: {e}")),
            }
        }
        Err("No terminal app was found (tried x-terminal-emulator, gnome-terminal, konsole, xfce4-terminal and xterm).".into())
    }
}

#[cfg(target_os = "macos")]
fn uuid_like() -> String {
    SessionId::new().to_string()
}

/// The computer's output volume, 0 to 100 (0 when muted). None where it can't be read: not
/// macOS, or an output device without a software volume (some displays and audio interfaces).
#[tauri::command]
pub async fn system_volume() -> CmdResult<Option<u32>> {
    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "set s to get volume settings",
                "-e",
                "(output volume of s as text) & \",\" & (output muted of s as text)",
            ])
            .output()
            .await
            .map_err(|e| format!("couldn't read the volume: {e}"))?;
        Ok(parse_volume_settings(&String::from_utf8_lossy(&out.stdout)))
    }
    #[cfg(not(target_os = "macos"))]
    Ok(None)
}

/// "24,false" (volume, muted) as the level to show.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_volume_settings(text: &str) -> Option<u32> {
    let (volume, muted) = text.trim().split_once(',')?;
    let volume: u32 = volume.trim().parse().ok()?;
    Some(if muted.trim() == "true" {
        0
    } else {
        volume.min(100)
    })
}

/// Sets the computer's output volume (0 to 100), unmuting it when above zero.
#[tauri::command]
pub async fn set_system_volume(volume: f64) -> CmdResult<()> {
    if !volume.is_finite() {
        return Err("invalid volume".into());
    }
    #[cfg(target_os = "macos")]
    {
        let level = volume.clamp(0.0, 100.0).round() as u32;
        let mut cmd = tokio::process::Command::new("/usr/bin/osascript");
        cmd.args(["-e", &format!("set volume output volume {level}")]);
        if level > 0 {
            cmd.args(["-e", "set volume output muted false"]);
        }
        let status = cmd
            .status()
            .await
            .map_err(|e| format!("couldn't set the volume: {e}"))?;
        if !status.success() {
            return Err("the system didn't accept the volume".into());
        }
    }
    Ok(())
}

/// Opens a link an agent printed: http(s) in the default browser (dev servers print
/// http://localhost ones), a `file://` one as [`open_local_file`] does.
#[tauri::command]
pub fn open_external(url: String) -> CmdResult<()> {
    if let Some(path) = url.strip_prefix("file://") {
        return open_local_file(&percent_decode(path));
    }
    let ok = (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() < 2048
        && !url.chars().any(|c| c.is_control() || c.is_whitespace());
    if !ok {
        return Err("only web links and local files can be opened".into());
    }
    open_url(&url).map_err(|e| format!("couldn't open the link: {e}"))
}

/// Opens the target of a Markdown link (`[label](target)`) the agent wrote, found by its label
/// in the session's transcript. Codex prints such links as underlined text with no target
/// (it only sends OSC 8 hyperlinks for web links), so the screen alone can't open them.
/// The newest link with that label wins. `false` when the transcript has no such link.
#[tauri::command]
pub fn open_transcript_link(transcript: String, label: String) -> CmdResult<bool> {
    let label = label.trim();
    if label.is_empty() || label.len() > 500 || !transcript.ends_with(".jsonl") {
        return Ok(false);
    }
    let Some(target) = transcript_link_target(&PathBuf::from(transcript), label) else {
        return Ok(false);
    };
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("file://")
    {
        return open_external(target).map(|_| true);
    }
    open_local_file(&target).map(|_| true)
}

/// The target of the newest `[label](target)` in the transcript's last few megabytes.
fn transcript_link_target(path: &std::path::Path, label: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 16 * 1024 * 1024;
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    // The transcript is JSON lines, so the link sits there JSON-escaped.
    let escaped = serde_json::to_string(label).ok()?;
    let needle = format!("[{}](", &escaped[1..escaped.len() - 1]);
    let start = text.rfind(&needle)? + needle.len();
    let rest = &text[start..];
    let raw = if let Some(inner) = rest.strip_prefix('<') {
        &inner[..inner.find('>')?]
    } else {
        // A title (`[a](url "title")`) or the end of the JSON string ends it too.
        &rest[..rest.find([')', ' ', '"'])?]
    };
    let target: String = serde_json::from_str(&format!("\"{raw}\"")).ok()?;
    (!target.is_empty() && target.len() < 4096 && !target.chars().any(char::is_control))
        .then_some(target)
}

/// File types that open in their default app; any other file is only shown in the file
/// manager, so a link can never run a program or script.
const VIEWABLE: &[&str] = &[
    "html", "htm", "md", "markdown", "txt", "log", "pdf", "png", "jpg", "jpeg", "gif", "webp",
    "svg", "csv", "tsv", "json", "jsonl", "xml", "yaml", "yml", "diff", "patch",
];

/// Opens a local file the agent linked to: a document in its default app, a folder in the
/// file manager, anything else revealed there. Paths may carry an editor-style
/// `:line[:col]` or `#L…` suffix, which is dropped when the file isn't found with it.
fn open_local_file(target: &str) -> CmdResult<()> {
    let expanded = match target.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .ok_or("no home folder")?,
        None => PathBuf::from(target),
    };
    if !expanded.is_absolute() {
        return Err("only absolute file paths can be opened".into());
    }
    let path = [expanded.clone(), strip_location(&expanded)]
        .into_iter()
        .find(|p| p.exists())
        .ok_or_else(|| format!("{} doesn't exist", expanded.display()))?;
    let viewable = path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| VIEWABLE.contains(&e.to_ascii_lowercase().as_str()));
    let result = if viewable || path.is_dir() {
        open_url(&path.to_string_lossy())
    } else {
        reveal_file(&path)
    };
    result.map_err(|e| format!("couldn't open the file: {e}"))
}

/// `/a/b.rs:12:3` → `/a/b.rs`, `/a/b.md#L4` → `/a/b.md`.
fn strip_location(path: &std::path::Path) -> PathBuf {
    let s = path.to_string_lossy();
    let s = s.split_once("#L").map_or(&*s, |(p, _)| p);
    let mut s = s;
    for _ in 0..2 {
        if let Some((head, tail)) = s.rsplit_once(':')
            && !tail.is_empty()
            && tail.bytes().all(|b| b.is_ascii_digit())
        {
            s = head;
        }
    }
    PathBuf::from(s)
}

/// Shows a file selected in Finder / Explorer (on Linux, opens its folder).
fn reveal_file(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("/usr/bin/open");
        cmd.arg("-R").arg(path);
        cmd
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = std::process::Command::new("explorer.exe");
        cmd.arg(format!("/select,{}", path.display()));
        cmd
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(path.parent().unwrap_or(path));
        cmd
    };
    cmd.spawn().map(|_| ())
}

/// Decodes `%XX` escapes in a `file://` URL's path.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn finds_newest_link_target_by_label() {
        let dir = std::env::temp_dir().join(format!("termlab-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("rollout.jsonl");
        std::fs::write(
            &f,
            concat!(
                r#"{"text":"see [Open the \"report\"](/old/r.html)"}"#,
                "\n",
                r#"{"text":"see [Open the \"report\"](/Volumes/S/r.html) and [x](<a b.md>)"}"#,
                "\n",
            ),
        )
        .unwrap();
        assert_eq!(
            transcript_link_target(&f, "Open the \"report\"").as_deref(),
            Some("/Volumes/S/r.html")
        );
        assert_eq!(transcript_link_target(&f, "x").as_deref(), Some("a b.md"));
        assert_eq!(transcript_link_target(&f, "missing"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn strips_editor_locations_and_decodes() {
        assert_eq!(
            strip_location("/a/b.rs:12:3".as_ref()),
            PathBuf::from("/a/b.rs")
        );
        assert_eq!(
            strip_location("/a/b.md#L4".as_ref()),
            PathBuf::from("/a/b.md")
        );
        assert_eq!(strip_location("/a/b.md".as_ref()), PathBuf::from("/a/b.md"));
        assert_eq!(percent_decode("/a%20b/c%2"), "/a b/c%2");
    }
}

/// Hands a link to the system's default browser. No shell is involved, so nothing in the
/// link is acted on.
pub fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("/usr/bin/open");
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = std::process::Command::new("rundll32.exe");
        cmd.arg("url.dll,FileProtocolHandler");
        cmd
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ())
}

/// macOS always has curl at this path. Windows 10 and later ship one too, and on Linux it
/// is found on the PATH when installed.
const CURL: &str = if cfg!(target_os = "macos") {
    "/usr/bin/curl"
} else {
    "curl"
};

/// The video a YouTube channel is live-streaming now, from its `/live` page's canonical link.
/// The sidebar's music stations use it when their stream restarted under a new video id.
/// (YouTube's own "embed a channel's live stream" no longer plays in embeds.)
#[tauri::command]
pub async fn youtube_live_video(channel: String) -> CmdResult<Option<String>> {
    let valid = channel.len() == 24
        && channel.starts_with("UC")
        && channel
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return Err("not a YouTube channel id".into());
    }
    let url = format!("https://www.youtube.com/channel/{channel}/live");
    let mut curl = tokio::process::Command::new(CURL);
    curl.args([
            "-sL",
            "--max-time",
            "10",
            "-A",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
            "-H",
            "Accept-Language: en",
            // Skips the cookie consent page (consent declined).
            "-H",
            "Cookie: SOCS=CAI",
            &url,
        ]);
    // Without this a console window flashes up on Windows (`CREATE_NO_WINDOW`).
    #[cfg(windows)]
    curl.creation_flags(0x0800_0000);
    let out = curl
        .output()
        .await
        .map_err(|e| format!("couldn't reach YouTube: {e}"))?;
    Ok(canonical_video_id(&String::from_utf8_lossy(&out.stdout)))
}

fn canonical_video_id(html: &str) -> Option<String> {
    const PREFIX: &str = r#"<link rel="canonical" href="https://www.youtube.com/watch?v="#;
    let rest = &html[html.find(PREFIX)? + PREFIX.len()..];
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    (id.len() == 11).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::{canonical_video_id, parse_volume_settings};

    #[test]
    fn reads_the_system_volume() {
        assert_eq!(parse_volume_settings("24,false\n"), Some(24));
        assert_eq!(parse_volume_settings("60,true\n"), Some(0));
        // An output device without a software volume.
        assert_eq!(parse_volume_settings("missing value,missing value\n"), None);
        assert_eq!(parse_volume_settings(""), None);
    }

    #[test]
    fn finds_the_live_video_on_a_channel_page() {
        let html = r#"<head><link rel="canonical" href="https://www.youtube.com/watch?v=rFZHOHl-L8A"><meta"#;
        assert_eq!(canonical_video_id(html).as_deref(), Some("rFZHOHl-L8A"));
        // Not live: the canonical link is the channel itself.
        let html = r#"<link rel="canonical" href="https://www.youtube.com/channel/UCSJ4gkVC6NrvII8umztf0Ow">"#;
        assert_eq!(canonical_video_id(html), None);
    }
}
