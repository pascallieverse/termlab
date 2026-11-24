//! Session labels: a name and a colour, made by the user and put on any number of sessions.
//!
//! One file in Termlab's data folder (`session-labels.json`): the labels, and which of them
//! each session carries. Like project notes, the app reads and writes it directly, so labels
//! work with any version of the background service and need no restart.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "session-labels.json";

/// The colours a label can have; the app gives each its shade.
pub const COLORS: &[&str] = &[
    "red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "gray",
];

/// The longest name a label can have, in characters.
pub const MAX_NAME: usize = 32;

/// Saves read the whole file and write it back: one at a time within a process.
static LOCK: Mutex<()> = Mutex::new(());

pub type LabelId = u32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub id: LabelId,
    pub name: String,
    /// One of [`COLORS`].
    pub color: String,
}

/// Every label, and the labels of each session that has any.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelBook {
    /// In the order they were made.
    #[serde(default)]
    pub labels: Vec<Label>,
    /// Session id → the ids of its labels, in the labels' own order.
    #[serde(default)]
    pub sessions: BTreeMap<String, Vec<LabelId>>,
}

/// What saving a label returns: the label as stored, and everything with it in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedLabel {
    pub label: Label,
    pub book: LabelBook,
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

fn load(dir: &Path) -> Result<LabelBook, String> {
    match std::fs::read(path(dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Couldn't read the session labels ({FILE}): {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LabelBook::default()),
        Err(e) => Err(format!("Couldn't read the session labels: {e}")),
    }
}

fn store(dir: &Path, book: &LabelBook) -> Result<(), String> {
    // Written alongside, then swapped in: a crash mid-save never leaves half a file.
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let temp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
    let json = serde_json::to_vec_pretty(book).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        std::fs::rename(&temp, path(dir))
    })();
    result.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("Couldn't save the session labels: {e}")
    })
}

pub fn read(dir: &Path) -> Result<LabelBook, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load(dir)
}

/// Makes a label (`id` None) or changes one's name and colour. Names are one line, at most
/// [`MAX_NAME`] characters, and no two labels share one (whatever the capitals).
pub fn save_label(
    dir: &Path,
    id: Option<LabelId>,
    name: &str,
    color: &str,
) -> Result<SavedLabel, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Give the label a name.".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!(
            "A label's name can be {MAX_NAME} characters at most."
        ));
    }
    if !COLORS.contains(&color) {
        return Err(format!("“{color}” isn't a label colour."));
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut book = load(dir)?;
    let lower = name.to_lowercase();
    if let Some(taken) = book
        .labels
        .iter()
        .find(|l| Some(l.id) != id && l.name.to_lowercase() == lower)
    {
        return Err(format!("There's already a label named “{}”.", taken.name));
    }
    let label = match id {
        Some(id) => {
            let label = book
                .labels
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("This label no longer exists.")?;
            label.name = name;
            label.color = color.to_string();
            label.clone()
        }
        None => {
            let label = Label {
                id: book.labels.iter().map(|l| l.id).max().unwrap_or(0) + 1,
                name,
                color: color.to_string(),
            };
            book.labels.push(label.clone());
            label
        }
    };
    store(dir, &book)?;
    Ok(SavedLabel { label, book })
}

/// Removes a label, from every session too.
pub fn delete_label(dir: &Path, id: LabelId) -> Result<LabelBook, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut book = load(dir)?;
    book.labels.retain(|l| l.id != id);
    for ids in book.sessions.values_mut() {
        ids.retain(|&l| l != id);
    }
    book.sessions.retain(|_, ids| !ids.is_empty());
    store(dir, &book)?;
    Ok(book)
}

/// Replaces one session's labels (none: the session is forgotten). Labels that don't exist
/// are left out.
pub fn set_session_labels(
    dir: &Path,
    session: &str,
    labels: &[LabelId],
) -> Result<LabelBook, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut book = load(dir)?;
    let ids: Vec<LabelId> = book
        .labels
        .iter()
        .map(|l| l.id)
        .filter(|id| labels.contains(id))
        .collect();
    if ids.is_empty() {
        book.sessions.remove(session);
    } else {
        book.sessions.insert(session.to_string(), ids);
    }
    store(dir, &book)?;
    Ok(book)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_made_changed_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(read(d).unwrap(), LabelBook::default());

        let bug = save_label(d, None, "  Bug  ", "red").unwrap().label;
        let client = save_label(d, None, "Client\n work", "blue").unwrap().label;
        assert_eq!((bug.id, bug.name.as_str()), (1, "Bug"));
        assert_eq!((client.id, client.name.as_str()), (2, "Client work"));

        // A name is taken whatever its capitals, except by the label that has it.
        assert!(save_label(d, None, "bug", "green").is_err());
        assert!(save_label(d, Some(client.id), "BUG", "green").is_err());
        let saved = save_label(d, Some(bug.id), "bug", "orange").unwrap();
        assert_eq!(saved.label.name, "bug");
        assert_eq!(saved.book.labels[0].color, "orange");
        assert_eq!(saved.book, read(d).unwrap());

        assert!(save_label(d, None, " ", "red").is_err());
        assert!(save_label(d, None, &"x".repeat(MAX_NAME + 1), "red").is_err());
        assert!(save_label(d, None, &"é".repeat(MAX_NAME), "red").is_ok());
        assert!(save_label(d, None, "Urgent", "magenta").is_err());
        assert!(save_label(d, Some(99), "Urgent", "red").is_err());

        let book = delete_label(d, client.id).unwrap();
        assert_eq!(book.labels.len(), 2);
        // No temp files are left behind.
        assert_eq!(std::fs::read_dir(d).unwrap().count(), 1);
    }

    #[test]
    fn sessions_carry_labels() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let a = save_label(d, None, "A", "red").unwrap().label.id;
        let b = save_label(d, None, "B", "blue").unwrap().label.id;

        // In the labels' order, once each, and only labels that exist.
        let book = set_session_labels(d, "s1", &[b, 42, a, b]).unwrap();
        assert_eq!(book.sessions["s1"], [a, b]);
        set_session_labels(d, "s2", &[b]).unwrap();

        // Removing a label takes it off every session; a session left with none is dropped.
        let book = delete_label(d, b).unwrap();
        assert_eq!(book.sessions.len(), 1);
        assert_eq!(book.sessions["s1"], [a]);

        // A new label doesn't take a number still in use.
        let c = save_label(d, None, "C", "green").unwrap().label.id;
        assert_ne!(c, a);

        let book = set_session_labels(d, "s1", &[]).unwrap();
        assert!(book.sessions.is_empty());
        assert_eq!(read(d).unwrap(), book);
    }

    #[test]
    fn a_damaged_file_is_reported_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "{not json").unwrap();
        assert!(read(dir.path()).is_err());
        assert!(save_label(dir.path(), None, "A", "red").is_err());
        assert!(set_session_labels(dir.path(), "s1", &[1]).is_err());
        assert!(delete_label(dir.path(), 1).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(FILE)).unwrap(),
            "{not json"
        );
    }
}
