//! What a session needs from the computer besides the screen: asking the person at it for
//! permission, keeping it awake, and the clipboard. One implementation per operating system.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Largest clipboard text handed to the browser.
pub const MAX_CLIPBOARD: usize = 1 << 20;

fn who(viewer: &str) -> &str {
    if viewer.is_empty() {
        "Someone signed in to your Termlab server"
    } else {
        viewer
    }
}

fn question(viewer: &str) -> String {
    format!(
        "Allow remote control of this computer?\n\n{}\nwants to see your screen and use your mouse and keyboard, through your Termlab server.",
        who(viewer)
    )
}

fn truncate(mut text: String) -> String {
    if text.len() > MAX_CLIPBOARD {
        let mut end = MAX_CLIPBOARD;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Runs `cmd`, waits up to `secs` (+5) seconds; its exit status and stdout.
async fn ask(mut cmd: Command, secs: u32) -> Option<(bool, String)> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(NO_WINDOW);
    let out = cmd.output();
    match tokio::time::timeout(Duration::from_secs(u64::from(secs) + 5), out).await {
        Ok(Ok(o)) => Some((
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )),
        _ => None,
    }
}

/// Feeds `text` to a clipboard command's stdin.
#[cfg_attr(windows, allow(dead_code))]
async fn pipe_to(mut cmd: Command, text: &str) -> bool {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(NO_WINDOW);
    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes()).await;
    }
    child.wait().await.map(|s| s.success()).unwrap_or(false)
}

#[cfg_attr(windows, allow(dead_code))]
async fn read_from(mut cmd: Command) -> Option<String> {
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(NO_WINDOW);
    let out = cmd.output().await.ok()?;
    out.status
        .success()
        .then(|| truncate(String::from_utf8_lossy(&out.stdout).into_owned()))
}

// ---- macOS -----------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub async fn confirm(viewer: &str, secs: u32) -> bool {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display dialog \"{}\" with title \"Termlab\" buttons {{\"Deny\", \"Allow\"}} default button \"Deny\" cancel button \"Deny\" with icon caution giving up after {secs}",
            esc(&question(viewer))
        );
        let mut cmd = Command::new("/usr/bin/osascript");
        cmd.arg("-e").arg(script);
        match ask(cmd, secs).await {
            Some((ok, text)) => {
                ok && text.contains("button returned:Allow") && !text.contains("gave up:true")
            }
            None => false,
        }
    }

    /// `caffeinate` keeps the display on and ends with this process.
    pub struct Awake(std::process::Child);

    impl Drop for Awake {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    pub fn keep_awake() -> Option<Awake> {
        std::process::Command::new("/usr/bin/caffeinate")
            .args(["-d", "-i", "-w", &std::process::id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
            .map(Awake)
    }

    /// Counts as user activity: no screensaver and no screen lock while someone is connected.
    pub fn declare_activity() {
        let _ = std::process::Command::new("/usr/bin/caffeinate")
            .args(["-u", "-t", "1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    pub async fn clipboard_get() -> Option<String> {
        read_from(Command::new("/usr/bin/pbpaste")).await
    }

    pub async fn clipboard_set(text: &str) -> bool {
        pipe_to(Command::new("/usr/bin/pbcopy"), text).await
    }
}

// ---- Windows ---------------------------------------------------------------------------

#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
mod imp {
    use super::*;

    /// PowerShell's `WScript.Shell.Popup`: Yes/No with a timeout (6 = Yes).
    pub async fn confirm(viewer: &str, secs: u32) -> bool {
        let esc = |s: &str| s.replace('\'', "''");
        let script = format!(
            "$r = (New-Object -ComObject WScript.Shell).Popup('{}', {secs}, 'Termlab', 4 + 48 + 256 + 4096); Write-Output $r",
            esc(&question(viewer))
        );
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        matches!(ask(cmd, secs).await, Some((true, out)) if out.trim() == "6")
    }

    /// Display and system stay on while a thread holds the request (it ends with the session).
    pub struct Awake(Option<std::sync::mpsc::Sender<()>>);

    impl Drop for Awake {
        fn drop(&mut self) {
            self.0.take();
        }
    }

    #[allow(unsafe_code)]
    pub fn keep_awake() -> Option<Awake> {
        use windows::Win32::System::Power::{
            ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
        };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                // SAFETY: plain Win32 call with valid flags; it applies to this thread.
                unsafe {
                    SetThreadExecutionState(
                        ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED,
                    );
                }
                // Until the sender is dropped.
                let _ = rx.recv();
                // SAFETY: as above.
                unsafe {
                    SetThreadExecutionState(ES_CONTINUOUS);
                }
            })
            .ok()?;
        Some(Awake(Some(tx)))
    }

    /// Injected input already counts as activity on Windows.
    pub fn declare_activity() {}

    pub async fn clipboard_get() -> Option<String> {
        tokio::task::spawn_blocking(clipboard::get)
            .await
            .ok()
            .flatten()
            .map(truncate)
    }

    pub async fn clipboard_set(text: &str) -> bool {
        let text = text.to_string();
        tokio::task::spawn_blocking(move || clipboard::set(&text))
            .await
            .unwrap_or(false)
    }

    #[allow(unsafe_code)]
    mod clipboard {
        use windows::Win32::Foundation::{HANDLE, HGLOBAL};
        use windows::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
        };
        use windows::Win32::System::Memory::{
            GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
        };
        use windows::Win32::System::Ole::CF_UNICODETEXT;

        /// Another program may hold the clipboard for a moment: try a few times.
        fn open() -> bool {
            for _ in 0..10 {
                // SAFETY: no owner window; paired with CloseClipboard.
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        }

        pub fn get() -> Option<String> {
            if !open() {
                return None;
            }
            // SAFETY: the clipboard is open; the handle is only used while locked.
            unsafe {
                let text = GetClipboardData(u32::from(CF_UNICODETEXT.0))
                    .ok()
                    .and_then(|h| {
                        let g = HGLOBAL(h.0);
                        let p = GlobalLock(g) as *const u16;
                        if p.is_null() {
                            return None;
                        }
                        let mut len = 0;
                        while *p.add(len) != 0 {
                            len += 1;
                        }
                        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
                        let _ = GlobalUnlock(g);
                        Some(s)
                    });
                let _ = CloseClipboard();
                text
            }
        }

        pub fn set(text: &str) -> bool {
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            if !open() {
                return false;
            }
            // SAFETY: the clipboard is open; the memory is ours until SetClipboardData takes it.
            unsafe {
                let ok = (|| {
                    EmptyClipboard().ok()?;
                    let g = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
                    let p = GlobalLock(g) as *mut u16;
                    if p.is_null() {
                        return None;
                    }
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                    let _ = GlobalUnlock(g);
                    SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(g.0))).ok()?;
                    Some(())
                })()
                .is_some();
                let _ = CloseClipboard();
                ok
            }
        }
    }
}

// ---- Linux -----------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    fn wayland() -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
    }

    /// zenity (GNOME and most others) or kdialog (KDE). Without either, no session: there's
    /// no one to ask.
    pub async fn confirm(viewer: &str, secs: u32) -> bool {
        let text = question(viewer);
        let mut zenity = Command::new("zenity");
        zenity.args([
            "--question",
            "--title=Termlab",
            "--ok-label=Allow",
            "--cancel-label=Deny",
            "--default-cancel",
            "--icon-name=dialog-warning",
            &format!("--timeout={secs}"),
            &format!("--text={text}"),
        ]);
        if let Some((ok, _)) = ask(zenity, secs).await {
            return ok;
        }
        let mut kdialog = Command::new("kdialog");
        kdialog.args(["--title", "Termlab", "--warningyesno", &text]);
        if let Some((ok, _)) = ask(kdialog, secs).await {
            return ok;
        }
        tracing::warn!("no zenity or kdialog to ask with: session refused");
        false
    }

    /// `systemd-inhibit` holds off sleep and idle; the screensaver is told over D-Bus.
    pub struct Awake {
        inhibit: Option<std::process::Child>,
        _bus: Option<zbus::blocking::Connection>,
    }

    impl Drop for Awake {
        fn drop(&mut self) {
            if let Some(mut c) = self.inhibit.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    pub fn keep_awake() -> Option<Awake> {
        let inhibit = std::process::Command::new("systemd-inhibit")
            .args([
                "--what=sleep:idle",
                "--who=Termlab",
                "--why=Someone is controlling this computer remotely",
                "--mode=block",
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
        // The screensaver's inhibition lasts while this connection is open.
        let bus = zbus::blocking::Connection::session().ok().inspect(|c| {
            let ok = c
                .call_method(
                    Some("org.freedesktop.ScreenSaver"),
                    "/org/freedesktop/ScreenSaver",
                    Some("org.freedesktop.ScreenSaver"),
                    "Inhibit",
                    &("Termlab", "Someone is controlling this computer remotely"),
                )
                .is_ok();
            if !ok {
                // GNOME: the session manager (8 = idle).
                let _ = c.call_method(
                    Some("org.gnome.SessionManager"),
                    "/org/gnome/SessionManager",
                    Some("org.gnome.SessionManager"),
                    "Inhibit",
                    &(
                        "Termlab",
                        0u32,
                        "Someone is controlling this computer remotely",
                        8u32,
                    ),
                );
            }
        });
        Some(Awake { inhibit, _bus: bus })
    }

    /// Injected input counts as activity; the inhibitors do the rest.
    pub fn declare_activity() {}

    pub async fn clipboard_get() -> Option<String> {
        if wayland() {
            let mut c = Command::new("wl-paste");
            c.arg("--no-newline");
            if let Some(t) = read_from(c).await {
                return Some(t);
            }
        }
        let mut c = Command::new("xclip");
        c.args(["-selection", "clipboard", "-o"]);
        if let Some(t) = read_from(c).await {
            return Some(t);
        }
        let mut c = Command::new("xsel");
        c.args(["--clipboard", "--output"]);
        read_from(c).await
    }

    pub async fn clipboard_set(text: &str) -> bool {
        if wayland() && pipe_to(Command::new("wl-copy"), text).await {
            return true;
        }
        let mut c = Command::new("xclip");
        c.args(["-selection", "clipboard"]);
        if pipe_to(c, text).await {
            return true;
        }
        let mut c = Command::new("xsel");
        c.args(["--clipboard", "--input"]);
        pipe_to(c, text).await
    }
}

// ---- elsewhere ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod imp {
    pub async fn confirm(_viewer: &str, _secs: u32) -> bool {
        false
    }
    pub struct Awake;
    pub fn keep_awake() -> Option<Awake> {
        None
    }
    pub fn declare_activity() {}
    pub async fn clipboard_get() -> Option<String> {
        None
    }
    pub async fn clipboard_set(_text: &str) -> bool {
        false
    }
}

pub use imp::{clipboard_get, clipboard_set, confirm, declare_activity, keep_awake};
