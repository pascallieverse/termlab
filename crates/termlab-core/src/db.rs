//! SQLite persistence for folders and sessions.
//!
//! Migrations are an ordered list of SQL scripts; `PRAGMA user_version` records how
//! many have been applied. Never edit a released migration — append a new one.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{CoreError, Result};
use crate::model::{
    AgentId, Billing, Folder, FolderId, Isolation, Millis, PermissionPreset, SessionId,
    SessionOrigin, SessionRecord, SessionStatus, TermSize,
};

const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    r#"
    CREATE TABLE folders (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        name        TEXT NOT NULL,
        path        TEXT NOT NULL UNIQUE,
        added_at    INTEGER NOT NULL
    );

    CREATE TABLE sessions (
        id                TEXT PRIMARY KEY,
        folder_id         INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
        agent             TEXT NOT NULL CHECK (agent IN ('claude','codex','vibe','gemini','aider')),
        title             TEXT NOT NULL,
        model             TEXT,
        permission_mode   TEXT,
        isolation         TEXT NOT NULL DEFAULT 'checkout',
        agent_session_id  TEXT,
        status            TEXT NOT NULL,
        exit_code         INTEGER,
        created_at        INTEGER NOT NULL,
        last_activity_at  INTEGER NOT NULL,
        billing           TEXT NOT NULL DEFAULT 'unknown'
    );

    CREATE INDEX sessions_folder ON sessions(folder_id);
    CREATE INDEX sessions_agent_session ON sessions(agent, agent_session_id);
    "#,
    // 2: Gemini CLI replaced by Antigravity CLI (Google ended Gemini CLI sign-in for
    //    individuals); data for the inspector, timeline and notifications. SQLite can't
    //    change a CHECK constraint in place, so the table is rebuilt.
    r#"
    CREATE TABLE sessions_v2 (
        id                TEXT PRIMARY KEY,
        folder_id         INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
        agent             TEXT NOT NULL CHECK (agent IN ('claude','codex','vibe','antigravity','aider')),
        title             TEXT NOT NULL,
        model             TEXT,
        permission_mode   TEXT,
        isolation         TEXT NOT NULL DEFAULT 'checkout',
        agent_session_id  TEXT,
        status            TEXT NOT NULL,
        exit_code         INTEGER,
        created_at        INTEGER NOT NULL,
        last_activity_at  INTEGER NOT NULL,
        billing           TEXT NOT NULL DEFAULT 'unknown',
        start_commit      TEXT,
        transcript_path   TEXT,
        notify            INTEGER NOT NULL DEFAULT 1
    );
    INSERT INTO sessions_v2 (id, folder_id, agent, title, model, permission_mode, isolation,
            agent_session_id, status, exit_code, created_at, last_activity_at, billing)
        SELECT id, folder_id, agent, title, model, permission_mode, isolation,
            agent_session_id, status, exit_code, created_at, last_activity_at, billing
        FROM sessions WHERE agent <> 'gemini';
    DROP TABLE sessions;
    ALTER TABLE sessions_v2 RENAME TO sessions;
    CREATE INDEX sessions_folder ON sessions(folder_id);
    CREATE INDEX sessions_agent_session ON sessions(agent, agent_session_id);

    CREATE TABLE hook_events (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        at          INTEGER NOT NULL,
        event       TEXT NOT NULL,
        summary     TEXT NOT NULL
    );
    CREATE INDEX hook_events_session ON hook_events(session_id, at);
    "#,
    // 3: sessions imported from an agent's history or attached to outside Termlab
    r#"
    ALTER TABLE sessions ADD COLUMN origin TEXT NOT NULL DEFAULT 'termlab';
    "#,
    // 4: sessions that ended because Termlab's background service stopped were recorded as
    //    errors; nothing failed, so they are "done" (recognisable by having no exit code).
    r#"
    UPDATE sessions SET status = 'done' WHERE status = 'error' AND exit_code IS NULL;
    "#,
    // 5: the terminal size a session's output was drawn at, to replay it correctly
    r#"
    ALTER TABLE sessions ADD COLUMN term_cols INTEGER;
    ALTER TABLE sessions ADD COLUMN term_rows INTEGER;
    "#,
    // 6: sessions to resume automatically when the background service starts. Sessions that
    //    already ended because the service stopped (done, no exit code) are included.
    r#"
    ALTER TABLE sessions ADD COLUMN resume_on_start INTEGER NOT NULL DEFAULT 0;
    UPDATE sessions SET resume_on_start = 1
        WHERE status = 'done' AND exit_code IS NULL AND origin <> 'imported';
    "#,
    // 7: the last name the agent gave the conversation (e.g. Claude's /rename) that Termlab
    //    applied, so it's applied once and a later rename in Termlab isn't overwritten.
    r#"
    ALTER TABLE sessions ADD COLUMN agent_title TEXT;
    "#,
    // 8: the colour given to the session inside the agent (Claude's /color).
    r#"
    ALTER TABLE sessions ADD COLUMN color TEXT;
    "#,
    // 9: the order of the project folders in the sidebar (drag to reorder). Existing folders
    //    keep the order they were added in.
    r#"
    ALTER TABLE folders ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
    UPDATE folders SET sort_order = id;
    "#,
];

/// Migration 9 adds `folders.sort_order`. Some databases already have that column (it was
/// first introduced by editing the initial schema, and added by hand to databases in use),
/// and SQLite can't add a column only if it's missing, so the step is skipped for those.
const FOLDER_ORDER_MIGRATION: usize = 9;

/// A hook event as stored for the Timeline tab.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEventRow {
    pub at: Millis,
    pub event: String,
    pub summary: String,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn schema_version(&self) -> Result<usize> {
        let v: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        Ok(v as usize)
    }

    fn migrate(&mut self) -> Result<()> {
        let current = self.schema_version()?;
        if current > MIGRATIONS.len() {
            return Err(CoreError::InvalidInput(format!(
                "database schema version {current} is newer than this Termlab ({})",
                MIGRATIONS.len()
            )));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            let tx = self.conn.transaction()?;
            let already_there = i + 1 == FOLDER_ORDER_MIGRATION
                && tx
                    .prepare(
                        "SELECT 1 FROM pragma_table_info('folders') WHERE name = 'sort_order'",
                    )?
                    .exists([])?;
            if !already_there {
                tx.execute_batch(sql)?;
            }
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
            tracing::info!(version = i + 1, "applied database migration");
        }
        Ok(())
    }

    // ---- folders ----------------------------------------------------------

    /// Adds a folder, or returns the existing row if this path is already known.
    pub fn add_folder(&self, name: &str, path: &Path, now: Millis) -> Result<Folder> {
        let path_str = path_to_str(path)?;
        if let Some(existing) = self.folder_by_path(path)? {
            return Ok(existing);
        }
        let next_order: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM folders",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO folders (name, path, added_at, sort_order) VALUES (?1, ?2, ?3, ?4)",
            params![name, path_str, now, next_order],
        )?;
        self.folder(self.conn.last_insert_rowid())
    }

    pub fn folder(&self, id: FolderId) -> Result<Folder> {
        self.conn
            .query_row(
                "SELECT id, name, path, added_at, sort_order FROM folders WHERE id = ?1",
                [id],
                folder_from_row,
            )
            .optional()?
            .ok_or(CoreError::FolderNotFound(id))
    }

    pub fn folder_by_path(&self, path: &Path) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, path, added_at, sort_order FROM folders WHERE path = ?1",
                [path_to_str(path)?],
                folder_from_row,
            )
            .optional()?)
    }

    pub fn list_folders(&self) -> Result<Vec<Folder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, added_at, sort_order FROM folders ORDER BY sort_order, id",
        )?;
        let rows = stmt.query_map([], folder_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Moves one folder before another (None appends), preserving all other ordering.
    pub fn move_folder(&mut self, id: FolderId, before: Option<FolderId>) -> Result<()> {
        self.folder(id)?;
        if let Some(target) = before {
            self.folder(target)?;
        }
        if before == Some(id) {
            return Ok(());
        }
        let mut ids: Vec<_> = self.list_folders()?.into_iter().map(|f| f.id).collect();
        ids.retain(|&item| item != id);
        let index = before
            .and_then(|target| ids.iter().position(|&item| item == target))
            .unwrap_or(ids.len());
        ids.insert(index, id);
        let tx = self.conn.transaction()?;
        for (order, id) in ids.into_iter().enumerate() {
            tx.execute(
                "UPDATE folders SET sort_order = ?1 WHERE id = ?2",
                params![order as i64, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes a folder and (via cascade) its session rows.
    pub fn remove_folder(&self, id: FolderId) -> Result<()> {
        let n = self
            .conn
            .execute("DELETE FROM folders WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(CoreError::FolderNotFound(id));
        }
        Ok(())
    }

    // ---- sessions ---------------------------------------------------------

    pub fn insert_session(&self, s: &SessionRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, folder_id, agent, title, model, permission_mode, isolation,
                agent_session_id, status, exit_code, created_at, last_activity_at, billing,
                start_commit, transcript_path, notify, origin, term_cols, term_rows)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                     ?18, ?19)",
            params![
                s.id.to_string(),
                s.folder_id,
                s.agent.as_str(),
                s.title,
                s.model,
                s.permission_mode.map(PermissionPreset::as_str),
                s.isolation.as_str(),
                s.agent_session_id,
                s.status.as_str(),
                s.exit_code,
                s.created_at,
                s.last_activity_at,
                s.billing.as_str(),
                s.start_commit,
                s.transcript_path,
                s.notify,
                s.origin.as_str(),
                s.term_size.map(|t| t.cols),
                s.term_size.map(|t| t.rows),
            ],
        )?;
        Ok(())
    }

    pub fn session(&self, id: SessionId) -> Result<SessionRecord> {
        self.conn
            .query_row(
                &format!("{SESSION_SELECT} WHERE id = ?1"),
                [id.to_string()],
                session_from_row,
            )
            .optional()?
            .ok_or(CoreError::SessionNotFound(id))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRecord>> {
        let mut stmt = self
            .conn
            .prepare(&format!("{SESSION_SELECT} ORDER BY created_at DESC"))?;
        let rows = stmt.query_map([], session_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_status(&self, id: SessionId, status: SessionStatus, now: Millis) -> Result<()> {
        self.expect_one(self.conn.execute(
            "UPDATE sessions SET status = ?2, last_activity_at = MAX(last_activity_at, ?3) WHERE id = ?1",
            params![id.to_string(), status.as_str(), now],
        )?, id)
    }

    pub fn set_exited(
        &self,
        id: SessionId,
        status: SessionStatus,
        exit_code: Option<i32>,
        now: Millis,
    ) -> Result<()> {
        self.expect_one(self.conn.execute(
            "UPDATE sessions SET status = ?2, exit_code = ?3, last_activity_at = MAX(last_activity_at, ?4)
             WHERE id = ?1",
            params![id.to_string(), status.as_str(), exit_code, now],
        )?, id)
    }

    pub fn set_agent_session_id(&self, id: SessionId, agent_session_id: &str) -> Result<()> {
        self.expect_one(
            self.conn.execute(
                "UPDATE sessions SET agent_session_id = ?2 WHERE id = ?1",
                params![id.to_string(), agent_session_id],
            )?,
            id,
        )
    }

    /// The last agent-given title Termlab applied to this session.
    pub fn agent_title(&self, id: SessionId) -> Result<Option<String>> {
        Ok(self.conn.query_row(
            "SELECT agent_title FROM sessions WHERE id = ?1",
            params![id.to_string()],
            |r| r.get(0),
        )?)
    }

    pub fn set_color(&self, id: SessionId, color: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET color = ?2 WHERE id = ?1",
            params![id.to_string(), color],
        )?;
        Ok(())
    }

    pub fn set_agent_title(&self, id: SessionId, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET agent_title = ?2 WHERE id = ?1",
            params![id.to_string(), title],
        )?;
        Ok(())
    }

    pub fn set_resume_on_start(&self, id: SessionId, on: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET resume_on_start = ?2 WHERE id = ?1",
            params![id.to_string(), on],
        )?;
        Ok(())
    }

    /// Sessions marked to resume, with the marks cleared (so a failing resume can't loop).
    pub fn take_sessions_to_resume(&self) -> Result<Vec<SessionRecord>> {
        let rows = {
            let mut stmt = self.conn.prepare(&format!(
                "{SESSION_SELECT} WHERE resume_on_start = 1 ORDER BY last_activity_at"
            ))?;
            stmt.query_map([], session_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        self.conn.execute(
            "UPDATE sessions SET resume_on_start = 0 WHERE resume_on_start = 1",
            [],
        )?;
        Ok(rows)
    }

    pub fn set_term_size(&self, id: SessionId, size: TermSize) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET term_cols = ?2, term_rows = ?3 WHERE id = ?1",
            params![id.to_string(), size.cols, size.rows],
        )?;
        Ok(())
    }

    pub fn set_transcript_path(&self, id: SessionId, path: &str) -> Result<()> {
        self.expect_one(
            self.conn.execute(
                "UPDATE sessions SET transcript_path = ?2 WHERE id = ?1",
                params![id.to_string(), path],
            )?,
            id,
        )
    }

    pub fn touch(&self, id: SessionId, now: Millis) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_activity_at = MAX(last_activity_at, ?2) WHERE id = ?1",
            params![id.to_string(), now],
        )?;
        Ok(())
    }

    /// The session tracking an agent's own session id, if any.
    pub fn session_by_agent_id(
        &self,
        agent: AgentId,
        agent_session_id: &str,
    ) -> Result<Option<SessionRecord>> {
        Ok(self
            .conn
            .query_row(
                &format!("{SESSION_SELECT} WHERE agent = ?1 AND agent_session_id = ?2 ORDER BY created_at DESC LIMIT 1"),
                params![agent.as_str(), agent_session_id],
                session_from_row,
            )
            .optional()?)
    }

    pub fn delete_session(&self, id: SessionId) -> Result<()> {
        self.expect_one(
            self.conn
                .execute("DELETE FROM sessions WHERE id = ?1", [id.to_string()])?,
            id,
        )
    }

    pub fn add_hook_event(
        &self,
        id: SessionId,
        at: Millis,
        event: &str,
        summary: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO hook_events (session_id, at, event, summary) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), at, event, summary],
        )?;
        Ok(())
    }

    pub fn hook_events(&self, id: SessionId) -> Result<Vec<HookEventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT at, event, summary FROM hook_events WHERE session_id = ?1 ORDER BY at, id",
        )?;
        let rows = stmt.query_map([id.to_string()], |r| {
            Ok(HookEventRow {
                at: r.get(0)?,
                event: r.get(1)?,
                summary: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_title(&self, id: SessionId, title: &str) -> Result<()> {
        self.expect_one(
            self.conn.execute(
                "UPDATE sessions SET title = ?2 WHERE id = ?1",
                params![id.to_string(), title],
            )?,
            id,
        )
    }

    /// Sessions still marked live when the background service starts belonged to a previous
    /// run of it and can't still be alive. They're marked as ended ("done", with no exit code:
    /// the agent didn't fail, Termlab stopped). Returns how many were changed.
    pub fn mark_orphans_ended(&self, now: Millis) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE sessions SET status = 'done', exit_code = NULL, resume_on_start = 1,
                 last_activity_at = MAX(last_activity_at, ?1)
             WHERE status NOT IN ('done', 'error')",
            [now],
        )?)
    }

    fn expect_one(&self, changed: usize, id: SessionId) -> Result<()> {
        if changed == 0 {
            Err(CoreError::SessionNotFound(id))
        } else {
            Ok(())
        }
    }
}

const SESSION_SELECT: &str =
    "SELECT id, folder_id, agent, title, model, permission_mode, isolation,
    agent_session_id, status, exit_code, created_at, last_activity_at, billing,
    start_commit, transcript_path, notify, origin, term_cols, term_rows, color FROM sessions";

fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        CoreError::InvalidInput(format!("path is not valid UTF-8: {}", path.display()))
    })
}

fn folder_from_row(r: &Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: r.get(0)?,
        name: r.get(1)?,
        path: PathBuf::from(r.get::<_, String>(2)?),
        added_at: r.get(3)?,
        sort_order: r.get(4)?,
    })
}

fn parse_col<T: FromStr>(r: &Row<'_>, idx: usize) -> rusqlite::Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let s: String = r.get(idx)?;
    s.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn session_from_row(r: &Row<'_>) -> rusqlite::Result<SessionRecord> {
    let permission_mode = r
        .get::<_, Option<String>>(5)?
        .map(|s| s.parse::<PermissionPreset>())
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?;
    Ok(SessionRecord {
        id: parse_col(r, 0)?,
        folder_id: r.get(1)?,
        agent: parse_col::<AgentId>(r, 2)?,
        title: r.get(3)?,
        model: r.get(4)?,
        permission_mode,
        isolation: parse_col::<Isolation>(r, 6)?,
        agent_session_id: r.get(7)?,
        status: parse_col::<SessionStatus>(r, 8)?,
        exit_code: r.get(9)?,
        created_at: r.get(10)?,
        last_activity_at: r.get(11)?,
        billing: parse_col::<Billing>(r, 12)?,
        start_commit: r.get(13)?,
        transcript_path: r.get(14)?,
        notify: r.get(15)?,
        origin: parse_col::<SessionOrigin>(r, 16)?,
        term_size: match (r.get::<_, Option<u16>>(17)?, r.get::<_, Option<u16>>(18)?) {
            (Some(cols), Some(rows)) if cols > 0 && rows > 0 => Some(TermSize { cols, rows }),
            _ => None,
        },
        color: r.get(19)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session(folder_id: FolderId) -> SessionRecord {
        SessionRecord {
            id: SessionId::new(),
            folder_id,
            agent: AgentId::Claude,
            title: "Fix the login bug".into(),
            model: Some("opus".into()),
            permission_mode: Some(PermissionPreset::AutoEdit),
            isolation: Isolation::Checkout,
            agent_session_id: None,
            status: SessionStatus::Running,
            exit_code: None,
            created_at: 1_000,
            last_activity_at: 1_000,
            billing: Billing::Subscription,
            start_commit: Some("abc123".into()),
            transcript_path: None,
            notify: true,
            origin: SessionOrigin::Termlab,
            term_size: None,
            color: None,
        }
    }

    #[test]
    fn migrations_apply_once_and_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("termlab.sqlite");
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len());
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len());
    }

    #[test]
    fn migration_4_turns_service_stops_into_done_but_keeps_real_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        {
            let db = Db::open(&path).unwrap();
            let f = db.add_folder("api", Path::new("/src/api"), 1).unwrap();
            let mut stopped = sample_session(f.id);
            stopped.status = SessionStatus::Error;
            stopped.exit_code = None;
            let mut failed = sample_session(f.id);
            failed.status = SessionStatus::Error;
            failed.exit_code = Some(2);
            db.insert_session(&stopped).unwrap();
            db.insert_session(&failed).unwrap();
            // Pretend the database predates migration 4.
            db.conn
                .execute_batch(
                    // Columns added by migrations after 3, so they can be re-applied.
                    "ALTER TABLE sessions DROP COLUMN term_cols;
                     ALTER TABLE sessions DROP COLUMN term_rows;
                     ALTER TABLE sessions DROP COLUMN resume_on_start;
                     ALTER TABLE sessions DROP COLUMN agent_title;
                     ALTER TABLE sessions DROP COLUMN color;",
                )
                .unwrap();
            db.conn.pragma_update(None, "user_version", 3).unwrap();
        }
        let db = Db::open(&path).unwrap();
        let all = db.list_sessions().unwrap();
        let by_code = |c: Option<i32>| all.iter().find(|s| s.exit_code == c).unwrap().status;
        assert_eq!(by_code(None), SessionStatus::Done);
        assert_eq!(by_code(Some(2)), SessionStatus::Error);
    }

    #[test]
    fn folders_crud() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_folder("api", Path::new("/src/api"), 1).unwrap();
        let b = db.add_folder("web", Path::new("/src/web"), 2).unwrap();
        assert_eq!((a.sort_order, b.sort_order), (0, 1));
        // Adding the same path again returns the existing folder.
        assert_eq!(db.add_folder("other", Path::new("/src/api"), 3).unwrap(), a);
        assert_eq!(db.list_folders().unwrap(), vec![a.clone(), b.clone()]);
        db.remove_folder(a.id).unwrap();
        assert_eq!(db.list_folders().unwrap(), vec![b]);
        assert!(matches!(
            db.remove_folder(a.id),
            Err(CoreError::FolderNotFound(_))
        ));
    }

    #[test]
    fn folder_order_is_added_to_older_databases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        {
            let db = Db::open(&path).unwrap();
            db.add_folder("a", Path::new("/a"), 1).unwrap();
            db.add_folder("b", Path::new("/b"), 2).unwrap();
            // A database from before folder ordering: no column, schema version 8.
            db.conn
                .execute_batch("ALTER TABLE folders DROP COLUMN sort_order;")
                .unwrap();
            db.conn.pragma_update(None, "user_version", 8).unwrap();
        }
        {
            let db = Db::open(&path).unwrap();
            assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len());
            let names: Vec<_> = db
                .list_folders()
                .unwrap()
                .into_iter()
                .map(|f| (f.name, f.sort_order))
                .collect();
            // Listed in the order they were added.
            assert_eq!(names, [("a".to_string(), 1), ("b".to_string(), 2)]);
            // A database that already had the column at version 8 (added by hand): the
            // step is skipped rather than failing, and the order it had is kept.
            db.conn
                .execute("UPDATE folders SET sort_order = 10 - id", [])
                .unwrap();
            db.conn.pragma_update(None, "user_version", 8).unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len());
        let names: Vec<_> = db
            .list_folders()
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, ["b", "a"]);
    }

    #[test]
    fn folder_order_survives_reopen_and_invalid_moves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("order.sqlite");
        let mut db = Db::open(&path).unwrap();
        let a = db.add_folder("a", Path::new("/a"), 1).unwrap().id;
        let b = db.add_folder("b", Path::new("/b"), 2).unwrap().id;
        let c = db.add_folder("c", Path::new("/c"), 3).unwrap().id;
        let ids = |db: &Db| {
            db.list_folders()
                .unwrap()
                .iter()
                .map(|f| f.id)
                .collect::<Vec<_>>()
        };
        db.move_folder(c, Some(a)).unwrap();
        assert_eq!(ids(&db), vec![c, a, b]);
        db.move_folder(c, None).unwrap();
        assert_eq!(ids(&db), vec![a, b, c]);
        db.move_folder(a, Some(c)).unwrap();
        assert_eq!(ids(&db), vec![b, a, c]);
        db.move_folder(a, Some(a)).unwrap();
        assert!(db.move_folder(a, Some(9999)).is_err());
        assert!(db.move_folder(9999, None).is_err());
        assert_eq!(ids(&db), vec![b, a, c]);
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(ids(&db), vec![b, a, c]);
        let d = db.add_folder("d", Path::new("/d"), 4).unwrap();
        assert_eq!(d.sort_order, 3);
        assert_eq!(ids(&db), vec![b, a, c, d.id]);
    }

    #[test]
    fn sessions_round_trip_and_update() {
        let db = Db::open_in_memory().unwrap();
        let f = db.add_folder("api", Path::new("/src/api"), 1).unwrap();
        let s = sample_session(f.id);
        db.insert_session(&s).unwrap();
        assert_eq!(db.session(s.id).unwrap(), s);

        db.set_agent_session_id(s.id, "abc").unwrap();
        db.set_exited(s.id, SessionStatus::Error, Some(2), 5_000)
            .unwrap();
        let got = db.session(s.id).unwrap();
        assert_eq!(got.agent_session_id.as_deref(), Some("abc"));
        assert_eq!(got.status, SessionStatus::Error);
        assert_eq!(got.exit_code, Some(2));
        assert_eq!(got.last_activity_at, 5_000);

        db.add_hook_event(s.id, 10, "Stop", "Waiting for input")
            .unwrap();
        assert_eq!(db.hook_events(s.id).unwrap().len(), 1);

        // Removing the folder removes its sessions.
        db.remove_folder(f.id).unwrap();
        assert!(db.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn orphaned_live_sessions_are_marked_ended() {
        let db = Db::open_in_memory().unwrap();
        let f = db.add_folder("api", Path::new("/src/api"), 1).unwrap();
        let live = sample_session(f.id);
        let mut done = sample_session(f.id);
        done.status = SessionStatus::Done;
        db.insert_session(&live).unwrap();
        db.insert_session(&done).unwrap();
        assert_eq!(db.mark_orphans_ended(9).unwrap(), 1);
        let orphan = db.session(live.id).unwrap();
        assert_eq!(orphan.status, SessionStatus::Done);
        assert_eq!(orphan.exit_code, None);
        assert_eq!(db.session(done.id).unwrap().status, SessionStatus::Done);
    }
}
