//! Tauri shell. Sessions live in `termlabd`; this side only relays requests, events and
//! terminal output, and owns window-level things (notifications, the dock badge).

mod commands;
mod daemon;
mod project_env;
mod ytmusic;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use termlab_core::DataDirs;

pub struct AppState {
    pub daemon: Arc<daemon::Daemon>,
    pub data_dir: PathBuf,
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let dirs = DataDirs::for_user()?;
            std::fs::create_dir_all(&dirs.root)?;
            tracing::info!(data_dir = %dirs.root.display(), "starting Termlab");
            let daemon = daemon::Daemon::new(dirs.clone());
            tauri::async_runtime::spawn(daemon.clone().run(app.handle().clone()));
            app.manage(AppState {
                daemon,
                data_dir: dirs.root,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            project_env::read_project_env,
            project_env::save_project_env,
            commands::daemon_call,
            commands::attach_session,
            commands::detach_session,
            commands::daemon_status,
            commands::open_in_terminal,
            commands::youtube_live_video,
            commands::open_external,
            commands::system_volume,
            commands::set_system_volume,
            ytmusic::youtube_music_dock,
            ytmusic::youtube_music_hide,
            ytmusic::youtube_music_navigate,
            ytmusic::youtube_music_volume,
            ytmusic::youtube_music_state,
            ytmusic::youtube_music_control,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Termlab");
}
