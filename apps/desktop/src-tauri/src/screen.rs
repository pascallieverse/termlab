//! Settings › Remote access › Screen sharing: switching it on and off, the macOS permissions
//! it needs, and the session running now (with a way to stop it).
//!
//! The switch is a file in Termlab's data folder (`termlab_core::screen`), written only here,
//! so the server can never turn it on. `termlabd` reports it to the server and starts
//! `termlab-screen` for each session; that helper stops by itself when it's switched off.
//!
//! macOS grants Screen Recording and Accessibility per app. The helper is started by the
//! background service, which this app started, so macOS asks for (and remembers) Termlab.

use serde::Serialize;
use tauri::State;
use termlab_core::screen::{self, ActiveSession, ScreenSettings};

use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenStatus {
    /// Screen sharing works on this operating system.
    supported: bool,
    enabled: bool,
    confirm: bool,
    /// macOS: Screen Recording is allowed for Termlab.
    screen_allowed: bool,
    /// macOS: Accessibility is allowed (mouse and keyboard).
    input_allowed: bool,
    active: Option<ActiveSession>,
}

#[tauri::command]
pub fn screen_status(state: State<'_, AppState>) -> ScreenStatus {
    let s = screen::load(&state.data_dir);
    ScreenStatus {
        supported: screen::supported(),
        enabled: s.enabled,
        confirm: s.confirm,
        screen_allowed: perms::screen_allowed(),
        input_allowed: perms::input_allowed(),
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
    if enabled && !perms::screen_allowed() {
        // Puts Termlab in System Settings' Screen Recording list (and asks, the first time).
        perms::request_screen();
    }
    Ok(screen_status(state))
}

/// Asks macOS for a permission and opens its page in System Settings.
#[tauri::command]
pub fn screen_request_permission(kind: String) -> Result<(), String> {
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
    #[cfg(not(unix))]
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

    pub fn screen_allowed() -> bool {
        // SAFETY: no arguments; reads this app's permission.
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn input_allowed() -> bool {
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

#[cfg(not(target_os = "macos"))]
mod perms {
    pub fn screen_allowed() -> bool {
        false
    }
    pub fn input_allowed() -> bool {
        false
    }
    pub fn request_screen() {}
    pub fn request_input() {}
}
