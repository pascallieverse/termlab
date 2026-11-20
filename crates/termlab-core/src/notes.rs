//! Project notes, shared by every session in a project folder.
//!
//! One file in Termlab's data folder (`project-notes.json`, keyed by the project's path).
//! The app reads and writes it directly, so notes work with any version of the background
//! service; the service reads it too, to sync notes with your Termlab server.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "project-notes.json";

/// Saves read the whole file and write it back: one at a time within a process.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectNotes {
    pub text: String,
    /// When they were last saved (ms since the epoch); None when there are none.
    pub updated_at: Option<i64>,
}

pub type AllNotes = BTreeMap<String, ProjectNotes>;

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// When the file last changed (None when there is none), to notice saves by the app.
pub fn modified(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(path(dir)).and_then(|m| m.modified()).ok()
}

fn load(dir: &Path) -> Result<AllNotes, String> {
    match std::fs::read(path(dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Couldn't read the project notes ({FILE}): {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AllNotes::new()),
        Err(e) => Err(format!("Couldn't read the project notes: {e}")),
    }
}

/// Every project's notes.
pub fn read_all(dir: &Path) -> Result<AllNotes, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load(dir)
}

pub fn read(dir: &Path, project: &str) -> Result<ProjectNotes, String> {
    Ok(read_all(dir)?.remove(project).unwrap_or_default())
}

/// Replaces one project's notes; blank text removes them.
pub fn write(dir: &Path, project: &str, text: String, now: i64) -> Result<ProjectNotes, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load(dir)?;
    let saved = if text.trim().is_empty() {
        all.remove(project);
        ProjectNotes::default()
    } else {
        let notes = ProjectNotes {
            text,
            updated_at: Some(now),
        };
        all.insert(project.to_string(), notes.clone());
        notes
    };
    // Written alongside, then swapped in: a crash mid-save never leaves half a file.
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let temp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
    let json = serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        std::fs::rename(&temp, path(dir))
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Couldn't save the project notes: {e}"));
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_per_project() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(read(d, "/src/api").unwrap(), ProjectNotes::default());
        assert!(modified(d).is_none());
        write(d, "/src/api", "first".into(), 10).unwrap();
        let saved = write(d, "/src/api", "- deploy on Fridays\n".into(), 20).unwrap();
        assert_eq!(saved.updated_at, Some(20));
        assert_eq!(read(d, "/src/api").unwrap(), saved);
        assert!(modified(d).is_some());
        // Each project has its own.
        write(d, "/src/web", "web".into(), 30).unwrap();
        assert_eq!(read(d, "/src/web").unwrap().text, "web");
        assert_eq!(read(d, "/src/api").unwrap(), saved);
        assert_eq!(read_all(d).unwrap().len(), 2);
        // Blank text removes them.
        write(d, "/src/api", " \n".into(), 40).unwrap();
        assert_eq!(read(d, "/src/api").unwrap(), ProjectNotes::default());
        assert_eq!(read(d, "/src/web").unwrap().text, "web");
        // No temp files are left behind.
        assert_eq!(std::fs::read_dir(d).unwrap().count(), 1);
    }

    #[test]
    fn a_damaged_file_is_reported_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "{not json").unwrap();
        assert!(read(dir.path(), "/a").is_err());
        assert!(write(dir.path(), "/a", "x".into(), 1).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(FILE)).unwrap(),
            "{not json"
        );
    }
}
