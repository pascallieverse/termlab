//! Project notes, shared by every session in a project folder.
//!
//! Kept by the app in one file in Termlab's data folder (`project-notes.json`, keyed by the
//! project's path; see `termlab_core::notes`), not through the background service: they work
//! with any version of it and need no restart, and they survive removing and re-adding the
//! folder. The service only reads and writes the same file to sync them with a Termlab
//! server.

use tauri::State;
use termlab_core::notes::{self, ProjectNotes};

use crate::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn read_project_notes(
    state: State<'_, AppState>,
    project: String,
) -> Result<ProjectNotes, String> {
    notes::read(&state.data_dir, &project)
}

#[tauri::command]
pub fn save_project_notes(
    state: State<'_, AppState>,
    project: String,
    text: String,
) -> Result<ProjectNotes, String> {
    notes::write(&state.data_dir, &project, text, now_ms())
}
