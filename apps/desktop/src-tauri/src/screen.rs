//! Settings › Remote access › Screen sharing: switching it on and off, the permissions it
//! needs (macOS: Screen Recording and Accessibility; Linux under Wayland: the desktop's
//! approval, once), and the session running now (with a way to stop it).
//!
//! The switch is a file in Termlab's data folder (`termlab_core::screen`), written only here,
//! so the server can never turn it on. `termlabd` reports it to the server and starts
//! `termlab-screen` for each session; that helper stops by itself when it's switched off.
//!
//! macOS grants Screen Recording and Accessibility per app. The helper is started by the
//! background service, which this app started, so macOS asks for (and remembers) Termlab.
//! Windows and X11 need nothing. Wayland's portal asks on screen the first time; the app
//! runs `termlab-screen authorize` so that happens while someone is at the computer, and the
//! portal's restore token lets later sessions start unattended.

use serde::Serialize;
use tauri::State;
use termlab_core::screen::{self, ActiveSession, ScreenSettings};

use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenStatus {
    /// Screen sharing works on this operating system.
    supported: bool,
    /// "macos", "windows" or "linux".
    os: &'static str,
    /// Linux: a Wayland session (the desktop's approval instead of system permissions).
    wayland: bool,
    enabled: bool,
    confirm: bool,
    /// macOS: Screen Recording is allowed for Termlab. Wayland: approved on the desktop.
    screen_allowed: bool,
    /// macOS: Accessibility is allowed (mouse and keyboard). Wayland: as above.
    input_allowed: bool,
    active: Option<ActiveSession>,
}

#[tauri::command]
pub fn screen_status(state: State<'_, AppState>) -> ScreenStatus {
    let s = screen::load(&state.data_dir);
    ScreenStatus {
        supported: screen::supported(),
        os: std::env::consts::OS,
        wayland: perms::wayland(),
        enabled: s.enabled,
        confirm: s.confirm,
        screen_allowed: perms::screen_allowed(&state.data_dir),
        input_allowed: perms::input_allowed(&state.data_dir),
        active: screen::active(&state.data_dir),
    }
}

#[tauri::command]
pub fn screen_set(
    state: State<'_, AppState>,
    enabled: bool,
    confirm: bool,
) -> Result<ScreenStatus, String> {
    if enabled && !screen::supported() {
        return Err("Screen sharing isn't available on this operating system yet.".into());
    }
    screen::save(&state.data_dir, ScreenSettings { enabled, confirm })
        .map_err(|e| e.to_string())?;
    if enabled && cfg!(target_os = "macos") && !perms::screen_allowed(&state.data_dir) {
        // Puts Termlab in System Settings' Screen Recording list (and asks, the first time).
        perms::request_screen();
    }
    Ok(screen_status(state))
}

/// macOS: asks for a permission and opens its page in System Settings. Linux under
/// Wayland: shows the desktop's screen sharing dialog now and keeps the approval.
#[tauri::command]
pub async fn screen_request_permission(
    state: State<'_, AppState>,
    kind: String,
) -> Result<(), String> {
    if cfg!(target_os = "linux") {
        let helper = crate::daemon::sibling_binary(screen::HELPER)
            .ok_or("The screen helper is missing from this installation.")?;
        let out = tokio::process::Command::new(helper)
            .arg("authorize")
            .arg(&state.data_dir)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let line = err
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("The desktop didn't approve screen sharing.");
            return Err(line.trim_start_matches("Error: ").to_string());
        }
        return Ok(());
    }
    let pane = match kind.as_str() {
        "screen" => {
            perms::request_screen();
            "Privacy_ScreenCapture"
        }
        "input" => {
            perms::request_input();
            "Privacy_Accessibility"
        }
        _ => return Err("unknown permission".into()),
    };
    #[cfg(target_os = "macos")]
    {
        let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
        std::process::Command::new("/usr/bin/open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = pane;
    Ok(())
}

/// Ends the screen session running now.
#[tauri::command]
pub fn screen_stop(state: State<'_, AppState>) -> Result<(), String> {
    let Some(active) = screen::active(&state.data_dir) else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        std::process::Command::new("/bin/kill")
            .args(["-TERM", &active.pid.to_string()])
            .status()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("taskkill")
            .args(["/F", "/PID", &active.pid.to_string()])
            .creation_flags(0x0800_0000)
            .status()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(unix, windows)))]
    let _ = active;
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod perms {
    //! The macOS permission checks: plain C functions from CoreGraphics and
    //! ApplicationServices, declared here (FFI, hence `unsafe`).
    use std::ffi::c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    pub fn wayland() -> bool {
        false
    }

    pub fn screen_allowed(_data_dir: &std::path::Path) -> bool {
        // SAFETY: no arguments; reads this app's permission.
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn input_allowed(_data_dir: &std::path::Path) -> bool {
        // SAFETY: no arguments; reads this app's permission.
        unsafe { AXIsProcessTrusted() }
    }

    pub fn request_screen() {
        // SAFETY: no arguments; shows macOS's prompt the first time.
        unsafe {
            CGRequestScreenCaptureAccess();
        }
    }

    pub fn request_input() {
        // SAFETY: a one-entry CFDictionary { kAXTrustedCheckOptionPrompt: true }, released
        // after the call.
        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let dict = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            );
            AXIsProcessTrustedWithOptions(dict);
            if !dict.is_null() {
                CFRelease(dict);
            }
        }
    }
}

/// Windows and X11: a program running as the user may capture and control its desktop.
/// Wayland: once the desktop's portal approved it (its restore token, saved by the helper).
#[cfg(not(target_os = "macos"))]
mod perms {
    use std::path::Path;

    pub fn wayland() -> bool {
        cfg!(target_os = "linux")
            && (std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland"))
    }

    fn approved(data_dir: &Path) -> bool {
        std::fs::read_to_string(data_dir.join("screen-portal.json"))
            .is_ok_and(|s| s.contains("restore_token"))
    }

    pub fn screen_allowed(data_dir: &Path) -> bool {
        !wayland() || approved(data_dir)
    }

    pub fn input_allowed(data_dir: &Path) -> bool {
        screen_allowed(data_dir)
    }

    pub fn request_screen() {}
    pub fn request_input() {}
}
