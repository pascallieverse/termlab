//! Updates of Termlab itself, through Tauri's updater: it reads `latest.json` from the
//! newest GitHub release, downloads the package for this platform, checks its signature
//! against the public key built into the app, and installs it.
//!
//! Only release builds made with `updater.conf.json` know where to look and which key to
//! trust (see docs/releasing.md). Other builds, development included, report that updates
//! aren't available rather than failing.
//!
//! Installing replaces the app and starts it again. Sessions belong to the background
//! service, so they keep running; that service is replaced separately (the "Restart
//! service" banner), except on Windows where the installer has to stop it.

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;

/// Whether this build was configured for updates.
pub struct Updates {
    enabled: bool,
}

/// Registers the updater when the build has its settings. The plugin refuses to start
/// without a public key, so it is left out otherwise.
pub fn setup(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let enabled = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|c| c.get("pubkey")?.as_str())
        .is_some_and(|key| !key.trim().is_empty());
    if enabled {
        app.handle()
            .plugin(tauri_plugin_updater::Builder::new().build())?;
    }
    app.manage(Updates { enabled });
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// False in builds that weren't set up for updates.
    supported: bool,
    current_version: String,
    /// The newer version, when there is one.
    version: Option<String>,
    notes: Option<String>,
}

#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    state: State<'_, Updates>,
) -> Result<UpdateCheck, String> {
    let mut result = UpdateCheck {
        supported: state.enabled,
        current_version: app.package_info().version.to_string(),
        version: None,
        notes: None,
    };
    if !state.enabled {
        return Ok(result);
    }
    let update = app
        .updater()
        .map_err(|e| format!("couldn't check for updates: {e}"))?
        .check()
        .await
        .map_err(|e| format!("couldn't check for updates: {e}"))?;
    if let Some(update) = update {
        tracing::info!(version = %update.version, "a Termlab update is available");
        result.version = Some(update.version);
        result.notes = update.body.filter(|notes| !notes.trim().is_empty());
    }
    Ok(result)
}

/// Downloads and installs the newest version, then starts it. Doesn't return when it
/// worked: the app is replaced.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, Updates>) -> Result<(), String> {
    if !state.enabled {
        return Err("This build of Termlab doesn't update itself.".into());
    }
    let update = app
        .updater()
        .map_err(|e| format!("couldn't check for updates: {e}"))?
        .check()
        .await
        .map_err(|e| format!("couldn't check for updates: {e}"))?
        .ok_or("Termlab is already up to date.")?;
    tracing::info!(version = %update.version, "installing a Termlab update");
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("couldn't install the update: {e}"))?;
    app.restart()
}
