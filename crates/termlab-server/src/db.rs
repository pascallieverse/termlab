//! The server's SQLite database. One connection behind a mutex: a personal server has
//! few writers, and every query here is small.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension};

const SCHEMA: &str = r#"
create table if not exists meta (key text primary key, value text not null);

create table if not exists owner (
  id integer primary key check (id = 1),
  password_hash text not null,
  totp_secret text not null,
  created_at text not null
);

create table if not exists web_sessions (
  id integer primary key,
  token_hash text not null unique,
  user_agent text,
  ip text,
  created_at text not null,
  last_seen_at text not null
);

create table if not exists machines (
  id integer primary key,
  name text not null,
  os text,
  version text,
  token_hash text not null unique,
  online integer not null default 0,
  last_seen_at text,
  created_at text not null
);

create table if not exists pairing_codes (
  code_hash text primary key,
  expires_at integer not null
);

create table if not exists sessions (
  id text primary key,
  machine_id integer not null references machines(id) on delete cascade,
  view text not null,
  status text not null,
  approval_key text,
  status_changed_at text,
  updated_at text not null
);

create table if not exists project_notes (
  machine_id integer not null references machines(id) on delete cascade,
  path text not null,
  text text not null,
  updated_at integer,
  primary key (machine_id, path)
);

create table if not exists audit_log (
  id integer primary key,
  action text not null,
  detail text,
  machine text,
  session_title text,
  ip text,
  created_at text not null
);

create table if not exists webhook_endpoints (
  id integer primary key,
  url text not null,
  description text,
  events text not null,
  secret text not null,
  enabled integer not null default 1,
  consecutive_failures integer not null default 0,
  last_delivery_at text,
  created_at text not null
);

create table if not exists webhook_deliveries (
  id integer primary key,
  endpoint_id integer not null references webhook_endpoints(id) on delete cascade,
  event_id text not null,
  event text not null,
  payload text not null,
  response_code text,
  response_body text,
  duration_ms integer,
  attempt integer not null default 0,
  ok integer not null default 0,
  created_at text not null,
  delivered_at text
);
create index if not exists deliveries_by_endpoint on webhook_deliveries(endpoint_id, id);

create table if not exists push_subscriptions (
  id integer primary key,
  endpoint text not null unique,
  p256dh text not null,
  auth text not null,
  label text,
  created_at text not null
);
"#;

pub struct Db(Mutex<Connection>);

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "pragma journal_mode = wal; pragma foreign_keys = on; pragma busy_timeout = 5000;",
        )?;
        conn.execute_batch(SCHEMA)?;
        // Machines are offline until they connect again after a restart.
        conn.execute("update machines set online = 0", [])?;
        Ok(Self(Mutex::new(conn)))
    }

    #[cfg(test)]
    pub fn memory() -> Self {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch("pragma foreign_keys = on;").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        Self(Mutex::new(conn))
    }

    /// Runs `f` with the connection.
    pub fn with<R>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<R>,
    ) -> rusqlite::Result<R> {
        let conn = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn)
    }

    pub fn has_owner(&self) -> bool {
        self.with(|c| {
            c.query_row("select count(*) from owner", [], |r| r.get::<_, i64>(0))
                .map(|n| n > 0)
        })
        .unwrap_or(false)
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.with(|c| {
            c.query_row("select value from meta where key = ?1", [key], |r| r.get(0))
                .optional()
        })
        .ok()
        .flatten()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.with(|c| {
            c.execute(
                "insert into meta (key, value) values (?1, ?2)
                 on conflict(key) do update set value = excluded.value",
                [key, value],
            )
            .map(|_| ())
        })
    }
}
