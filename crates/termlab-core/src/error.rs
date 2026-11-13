use std::path::PathBuf;

use crate::model::{AgentId, SessionId};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("could not start terminal: {0}")]
    Pty(String),

    #[error("no such session: {0}")]
    SessionNotFound(SessionId),

    #[error("session {0} is not running")]
    SessionNotLive(SessionId),

    #[error("no such folder: {0}")]
    FolderNotFound(i64),

    #[error("not a directory: {}", .0.display())]
    NotADirectory(PathBuf),

    #[error("{} is not installed (not found on PATH)", .0.display_name())]
    AgentNotInstalled(AgentId),

    #[error("could not find the app data directory")]
    NoDataDir,

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;
