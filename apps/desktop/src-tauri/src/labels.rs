//! Session labels: coloured names the user makes and puts on sessions.
//!
//! Kept by the app in one file in Termlab's data folder (`session-labels.json`; see
//! `termlab_core::labels`), not through the background service: they work with any version of
//! it and need no restart.

use tauri::State;
use termlab_core::labels::{self, LabelBook, LabelId, SavedLabel};
use termlab_core::model::SessionId;

use crate::AppState;

#[tauri::command]
pub fn read_labels(state: State<'_, AppState>) -> Result<LabelBook, String> {
    labels::read(&state.data_dir)
}

/// Makes a label (`id` null) or changes one.
#[tauri::command]
pub fn save_label(
    state: State<'_, AppState>,
    id: Option<LabelId>,
    name: String,
    color: String,
) -> Result<SavedLabel, String> {
    labels::save_label(&state.data_dir, id, &name, &color)
}

#[tauri::command]
pub fn delete_label(state: State<'_, AppState>, id: LabelId) -> Result<LabelBook, String> {
    labels::delete_label(&state.data_dir, id)
}

/// Replaces one session's labels.
#[tauri::command]
pub fn set_session_labels(
    state: State<'_, AppState>,
    session: SessionId,
    labels: Vec<LabelId>,
) -> Result<LabelBook, String> {
    labels::set_session_labels(&state.data_dir, &session.to_string(), &labels)
}
