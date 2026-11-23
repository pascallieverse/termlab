//! Remote screen sharing settings, shared by the app (which changes them), `termlabd`
//! (which reports them to the server and starts sessions) and `termlab-screen` (which stops
//! a session when sharing is switched off).
//!
//! Kept in `<data-dir>/screen.json`. Only the app on this computer writes it: the server
//! can't switch sharing on. While a session runs, `termlab-screen` keeps
//! `<data-dir>/screen-session.json` (`ActiveSession`), so the app can show it and stop it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "screen.json";
pub const ACTIVE_FILE: &str = "screen-session.json";
/// The helper binary, next to `termlabd`.
pub const HELPER: &str = "termlab-screen";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenSettings {
    /// Others signed in to your Termlab server may see and control this computer's screen.
    #[serde(default)]
    pub enabled: bool,
    /// Ask on this computer before each session.
    #[serde(default)]
    pub confirm: bool,
}

/// The session running now, as `termlab-screen` writes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSession {
    pub pid: u32,
    /// ms since the epoch
    #[serde(alias = "started_at")]
    pub started_at: u64,
    #[serde(default)]
    pub viewer: String,
}

/// Screen sharing works on this operating system.
pub const fn supported() -> bool {
    cfg!(target_os = "macos")
}

pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

pub fn load(dir: &Path) -> ScreenSettings {
    std::fs::read(settings_path(dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, s: ScreenSettings) -> std::io::Result<()> {
    let tmp = dir.join(format!("{SETTINGS_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&s).unwrap_or_default())?;
    std::fs::rename(tmp, settings_path(dir))
}

/// The session running now, if its process is still alive.
pub fn active(dir: &Path) -> Option<ActiveSession> {
    let a: ActiveSession = std::fs::read(dir.join(ACTIVE_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())?;
    alive(a.pid).then_some(a)
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(not(unix))]
fn alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_off_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), ScreenSettings::default());
        let on = ScreenSettings {
            enabled: true,
            confirm: true,
        };
        save(dir.path(), on).unwrap();
        assert_eq!(load(dir.path()), on);
    }

    #[test]
    fn a_session_whose_process_is_gone_isnt_active() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(ACTIVE_FILE),
            r#"{"pid":999999,"started_at":1,"viewer":"x"}"#,
        )
        .unwrap();
        assert!(active(dir.path()).is_none());
        std::fs::write(
            dir.path().join(ACTIVE_FILE),
            format!(
                r#"{{"pid":{},"started_at":1,"viewer":"x"}}"#,
                std::process::id()
            ),
        )
        .unwrap();
        assert_eq!(active(dir.path()).unwrap().viewer, "x");
    }
}
