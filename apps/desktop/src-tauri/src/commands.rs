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

/// Opens a web link in the user's default browser. Only plain http(s) links: the terminal
/// opens links agents print, and dev servers print http://localhost ones.
#[tauri::command]
pub fn open_external(url: String) -> CmdResult<()> {
    let ok = (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() < 2048
        && !url.chars().any(|c| c.is_control() || c.is_whitespace());
    if !ok {
        return Err("only http and https links can be opened".into());
    }
    open_url(&url).map_err(|e| format!("couldn't open the link: {e}"))
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
