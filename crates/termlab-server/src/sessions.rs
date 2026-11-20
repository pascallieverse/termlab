//! Session state as machines report it, and the events derived from changes. Termlab
//! sends whole views, never transitions, so the server compares old and new here.

use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::util::{clip, now};
use crate::{AppRef, notify};

pub const STATUSES: [&str; 5] = ["needs_you", "running", "idle", "done", "error"];

#[derive(Debug, Deserialize)]
pub struct ApprovalOptionIn {
    pub choice: Option<String>,
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalIn {
    pub kind: Option<String>,
    pub question: String,
    pub detail: Option<String>,
    #[serde(default)]
    pub options: Vec<ApprovalOptionIn>,
}

#[derive(Debug, Deserialize)]
pub struct ChangedFileIn {
    pub path: String,
    #[serde(default)]
    pub added: u64,
    #[serde(default)]
    pub removed: u64,
}

/// One session as a machine sends it. Everything is bounded before it's stored.
#[derive(Debug, Deserialize)]
pub struct SessionIn {
    pub id: String,
    pub project: String,
    /// The project folder's path on the machine; also what its notes are keyed by.
    pub directory: Option<String>,
    /// Where the project sits in Termlab's sidebar.
    pub project_order: Option<i64>,
    /// The project folder's id on the machine.
    pub folder_id: Option<i64>,
    /// Put on hold in Termlab (its On hold tab).
    #[serde(default)]
    pub on_hold: bool,
    /// When the session started (ms since the epoch); the sidebar lists newest first.
    pub created_at: Option<i64>,
    pub title: String,
    pub agent: String,
    pub model: Option<String>,
    pub branch: Option<String>,
    pub status: String,
    pub approval: Option<ApprovalIn>,
    #[serde(default)]
    pub preview: Vec<Option<String>>,
    pub tokens: Option<u64>,
    pub context_pct: Option<u8>,
    #[serde(default)]
    pub changed_files: Vec<ChangedFileIn>,
}

impl SessionIn {
    /// Checks and bounds the view; `None` when it isn't usable.
    fn clean(self) -> Option<(String, Value)> {
        let id = uuid::Uuid::parse_str(&self.id).ok()?.to_string();
        if !STATUSES.contains(&self.status.as_str()) {
            return None;
        }
        let approval = self.approval.map(|a| {
            let kind = match a.kind.as_deref() {
                Some("question") => "question",
                _ => "permission",
            };
            json!({
                "kind": kind,
                "question": clip(&a.question, 500),
                "detail": a.detail.map(|d| clip(&d, 2000)),
                "options": a.options.into_iter().take(10).map(|o| json!({
                    "choice": o.choice.filter(|c| matches!(c.as_str(), "allow_once" | "allow_always" | "deny")),
                    "label": clip(&o.label, 200),
                })).collect::<Vec<_>>(),
            })
        });
        let preview: Vec<String> = self
            .preview
            .into_iter()
            .rev()
            .take(12)
            .rev()
            .map(|l| clip(&l.unwrap_or_default(), 500))
            .collect();
        Some((
            id.clone(),
            json!({
                "id": id,
                "project": clip(&self.project, 200),
                "directory": self.directory.map(|d| clip(&d, 1000)),
                "project_order": self.project_order,
                "folder_id": self.folder_id,
                "on_hold": self.on_hold,
                "created_at": self.created_at,
                "title": clip(&self.title, 300),
                "agent": clip(&self.agent, 40),
                "model": self.model.map(|m| clip(&m, 100)),
                "branch": self.branch.map(|b| clip(&b, 200)),
                "status": self.status,
                "approval": approval,
                "preview": preview,
                "tokens": self.tokens,
                "context_pct": self.context_pct.map(|p| p.min(100)),
                "changed_files": self.changed_files.into_iter().take(200).map(|f| json!({
                    "path": clip(&f.path, 500), "added": f.added, "removed": f.removed,
                })).collect::<Vec<_>>(),
            }),
        ))
    }
}

fn approval_key(view: &Value) -> Option<String> {
    let a = view.get("approval").filter(|a| !a.is_null())?;
    Some(format!(
        "{}|{}|{}",
        a["kind"].as_str().unwrap_or(""),
        a["question"].as_str().unwrap_or(""),
        a["detail"].as_str().unwrap_or("")
    ))
}

/// Which events a change produces.
pub fn transitions(
    old: Option<(&str, Option<&str>)>,
    new_status: &str,
    new_approval: Option<&str>,
    from_snapshot: bool,
) -> Vec<&'static str> {
    let mut events = Vec::new();
    if old.is_none() && !from_snapshot {
        events.push("session.started");
    }
    let changed = old.is_none_or(|(s, _)| s != new_status);
    if changed && new_status == "needs_you" {
        events.push("session.needs_you");
    }
    if let Some(key) = new_approval
        && old.and_then(|(_, a)| a) != Some(key)
    {
        events.push("approval.requested");
    }
    if changed && new_status == "done" {
        events.push("session.done");
    }
    if changed && new_status == "error" {
        events.push("session.error");
    }
    events
}

/// The stored view plus the fields the server fills in.
fn full_view(
    view: &str,
    machine_id: i64,
    machine: &str,
    changed_at: Option<String>,
    updated_at: String,
) -> Value {
    let mut v: Value = serde_json::from_str(view).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut v {
        map.insert("machine_id".into(), json!(machine_id));
        map.insert("machine".into(), json!(machine));
        map.insert("status_changed_at".into(), json!(changed_at));
        map.insert("updated_at".into(), json!(updated_at));
    }
    v
}

/// Stores one reported session, broadcasts it and fires its events.
pub fn upsert(app: &AppRef, machine_id: i64, raw: Value, from_snapshot: bool) -> Option<String> {
    let input: SessionIn = serde_json::from_value(raw).ok()?;
    let (id, view) = input.clean()?;
    let status = view["status"].as_str().unwrap_or("idle").to_string();
    let akey = approval_key(&view);
    let t = now();

    let result = app.db.with(|c| {
        let owner: Option<i64> = c
            .query_row("select machine_id from sessions where id = ?1", [&id], |r| r.get(0))
            .optional()?;
        if owner.is_some_and(|m| m != machine_id) {
            // Another machine's session id: refuse rather than take it over.
            return Ok(None);
        }
        let old: Option<(String, Option<String>, Option<String>)> = c
            .query_row(
                "select status, approval_key, status_changed_at from sessions where id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let changed_at = match &old {
            Some((s, _, at)) if *s == status => at.clone(),
            _ => Some(t.clone()),
        };
        let events = transitions(
            old.as_ref().map(|(s, a, _)| (s.as_str(), a.as_deref())),
            &status,
            akey.as_deref(),
            from_snapshot,
        );
        c.execute(
            "insert into sessions (id, machine_id, view, status, approval_key, status_changed_at, updated_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             on conflict(id) do update set view = excluded.view, status = excluded.status,
               approval_key = excluded.approval_key, status_changed_at = excluded.status_changed_at,
               updated_at = excluded.updated_at",
            (&id, machine_id, view.to_string(), &status, &akey, &changed_at, &t),
        )?;
        let machine: String = c.query_row("select name from machines where id = ?1", [machine_id], |r| r.get(0))?;
        Ok(Some((events, machine, changed_at)))
    });

    let (events, machine, changed_at) = match result {
        Ok(Some(x)) => x,
        Ok(None) => return None,
        Err(e) => {
            tracing::error!(error = %e, "storing a session failed");
            return None;
        }
    };
    let full = full_view(&view.to_string(), machine_id, &machine, changed_at, t);
    app.hub
        .broadcast(&json!({ "type": "session.updated", "session": full }));
    for event in events {
        notify::event(app, event, &full);
    }
    Some(id)
}

/// Replaces a machine's sessions with the given list.
pub fn snapshot(app: &AppRef, machine_id: i64, list: Vec<Value>) {
    let keep: Vec<String> = list
        .into_iter()
        .take(500)
        .filter_map(|v| upsert(app, machine_id, v, true))
        .collect();
    let gone: Vec<String> = app
        .db
        .with(|c| {
            let mut st = c.prepare("select id from sessions where machine_id = ?1")?;
            let ids = st
                .query_map([machine_id], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ids)
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|id| !keep.contains(id))
        .collect();
    for id in gone {
        remove(app, machine_id, &id);
    }
}

pub fn remove(app: &AppRef, machine_id: i64, id: &str) {
    let n = app
        .db
        .with(|c| {
            c.execute(
                "delete from sessions where id = ?1 and machine_id = ?2",
                (id, machine_id),
            )
        })
        .unwrap_or(0);
    if n > 0 {
        app.hub
            .broadcast(&json!({ "type": "session.removed", "id": id }));
    }
}

/// Every session, for the web UI.
pub fn all(app: &AppRef) -> rusqlite::Result<Vec<Value>> {
    app.db.with(|c| {
        let mut st = c.prepare(
            "select s.view, s.machine_id, m.name, s.status_changed_at, s.updated_at
             from sessions s join machines m on m.id = s.machine_id
             order by json_extract(s.view, '$.project'), s.updated_at",
        )?;
        let rows = st.query_map([], |r| {
            Ok(full_view(
                &r.get::<_, String>(0)?,
                r.get(1)?,
                &r.get::<_, String>(2)?,
                r.get(3)?,
                r.get(4)?,
            ))
        })?;
        rows.collect()
    })
}

/// One session with its machine id, for commands.
pub fn find(app: &AppRef, id: &str) -> rusqlite::Result<Option<(Value, i64, String)>> {
    app.db.with(|c| {
        c.query_row(
            "select s.view, s.machine_id, m.name from sessions s join machines m on m.id = s.machine_id where s.id = ?1",
            [id],
            |r| {
                let view: String = r.get(0)?;
                Ok((serde_json::from_str(&view).unwrap_or(Value::Null), r.get(1)?, r.get(2)?))
            },
        )
        .optional()
    })
}

/// A machine as the web UI shows it.
pub fn machine_view(app: &AppRef, id: i64) -> rusqlite::Result<Option<Value>> {
    Ok(machines(app)?.into_iter().find(|m| m["id"] == json!(id)))
}

pub fn machines(app: &AppRef) -> rusqlite::Result<Vec<Value>> {
    app.db.with(|c| {
        let mut st = c.prepare(
            "select m.id, m.name, m.os, m.version, m.online, m.last_seen_at,
                    (select count(*) from sessions s where s.machine_id = m.id),
                    (select count(*) from sessions s where s.machine_id = m.id and s.status in ('running', 'needs_you')),
                    exists(select 1 from meta where key = 'notes_sync:' || m.id)
             from machines m order by m.name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "os": r.get::<_, Option<String>>(2)?,
                "version": r.get::<_, Option<String>>(3)?,
                "online": r.get::<_, i64>(4)? != 0,
                "last_seen_at": r.get::<_, Option<String>>(5)?,
                "sessions": r.get::<_, i64>(6)?,
                "running": r.get::<_, i64>(7)?,
                // Its Termlab syncs project notes (0.2.9 and newer).
                "notes_sync": r.get::<_, bool>(8)?,
            }))
        })?;
        rows.collect()
    })
}

// ---- plan usage ------------------------------------------------------------------------

/// The plan usage a machine reported (Termlab's `PlanUsage` list), bounded field by field.
pub fn clean_usage(raw: &Value) -> Value {
    let list = raw.as_array().cloned().unwrap_or_default();
    let s = |v: &Value, max: usize| v.as_str().map(|t| clip(t, max));
    Value::Array(
        list.iter()
            .take(10)
            .filter_map(|u| {
                let agent = s(&u["agent"], 40)?;
                let windows: Vec<Value> = u["windows"]
                    .as_array()
                    .map(|w| {
                        w.iter().take(12).filter_map(|w| {
                        Some(json!({
                            "id": s(&w["id"], 60)?,
                            "label": s(&w["label"], 100),
                            "usedPercentage": w["usedPercentage"].as_f64()?.clamp(0.0, 1000.0),
                            "resetsAt": w["resetsAt"].as_i64(),
                        }))
                    }).collect()
                    })
                    .unwrap_or_default();
                Some(json!({
                    "agent": agent,
                    "windows": windows,
                    "updatedAt": u["updatedAt"].as_i64().unwrap_or(0),
                    "connected": u["connected"].as_bool(),
                    "plan": s(&u["plan"], 60),
                    "note": s(&u["note"], 300),
                }))
            })
            .collect(),
    )
}

pub fn set_usage(app: &AppRef, machine_id: i64, raw: &Value) -> Value {
    let usage = clean_usage(raw);
    let _ = app
        .db
        .set_meta(&format!("usage:{machine_id}"), &usage.to_string());
    usage
}

/// Each machine's last reported plan usage.
pub fn usage(app: &AppRef) -> rusqlite::Result<Vec<Value>> {
    app.db.with(|c| {
        let mut st = c.prepare(
            "select m.id, meta.value from machines m join meta on meta.key = 'usage:' || m.id",
        )?;
        let rows = st.query_map([], |r| {
            let v: String = r.get(1)?;
            Ok(json!({
                "machine_id": r.get::<_, i64>(0)?,
                "usage": serde_json::from_str::<Value>(&v).unwrap_or(json!([])),
            }))
        })?;
        rows.collect()
    })
}

// ---- projects and agents ---------------------------------------------------------------

/// A machine's project folders (in its sidebar order) and installed agents, bounded.
pub fn set_projects(app: &AppRef, machine_id: i64, msg: &Value) -> Value {
    let s = |v: &Value, max: usize| v.as_str().map(|t| clip(t, max));
    let projects: Vec<Value> = msg["projects"]
        .as_array()
        .map(|l| {
            l.iter()
                .take(500)
                .filter_map(|p| {
                    Some(json!({
                        "id": p["id"].as_i64()?,
                        "name": s(&p["name"], 200)?,
                        "path": s(&p["path"], 1000)?,
                        "order": p["order"].as_i64().unwrap_or(0),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    let agents: Vec<Value> = msg["agents"]
        .as_array()
        .map(|l| {
            l.iter()
                .take(20)
                .filter_map(|a| {
                    Some(json!({
                        "agent": s(&a["agent"], 40)?,
                        "name": s(&a["name"], 60)?,
                        "model_flag": a["model_flag"].as_bool().unwrap_or(false),
                        "models": a["models"].as_array().map(|m| m.iter().take(20).filter_map(|x| s(x, 100)).collect::<Vec<_>>()).unwrap_or_default(),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    let v = json!({ "machine_id": machine_id, "projects": projects, "agents": agents });
    let _ = app
        .db
        .set_meta(&format!("projects:{machine_id}"), &v.to_string());
    v
}

/// Each machine's projects and agents.
pub fn projects(app: &AppRef) -> rusqlite::Result<Vec<Value>> {
    app.db.with(|c| {
        let mut st = c.prepare(
            "select meta.value from machines m join meta on meta.key = 'projects:' || m.id",
        )?;
        let rows = st.query_map([], |r| {
            let v: String = r.get(0)?;
            Ok(serde_json::from_str::<Value>(&v).unwrap_or(Value::Null))
        })?;
        rows.collect()
    })
}

/// One machine's stored projects and agents (empty when it never sent them).
pub fn machine_projects(app: &AppRef, machine_id: i64) -> Value {
    app.db
        .meta(&format!("projects:{machine_id}"))
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_else(|| json!({ "projects": [], "agents": [] }))
}

// ---- project notes ---------------------------------------------------------------------

/// The longest notes kept for one project, in characters.
pub const NOTES_MAX: usize = 100_000;

#[derive(Debug, Deserialize)]
pub struct NotesIn {
    pub path: String,
    pub text: String,
    pub updated_at: Option<i64>,
}

/// A machine sent all of its projects' notes: they replace what was stored. Returns the
/// machine's notes as browsers get them.
pub fn replace_notes(
    app: &AppRef,
    machine_id: i64,
    list: Vec<Value>,
) -> rusqlite::Result<Vec<Value>> {
    let list: Vec<NotesIn> = list
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .take(1000)
        .collect();
    app.db.with(|c| {
        let tx = c.unchecked_transaction()?;
        tx.execute("delete from project_notes where machine_id = ?1", [machine_id])?;
        for n in &list {
            if n.path.is_empty() || n.text.trim().is_empty() {
                continue;
            }
            tx.execute(
                "insert or replace into project_notes (machine_id, path, text, updated_at) values (?1, ?2, ?3, ?4)",
                (machine_id, clip(&n.path, 1000), clip(&n.text, NOTES_MAX), n.updated_at),
            )?;
        }
        tx.execute(
            "insert or ignore into meta (key, value) values (?1, '1')",
            [format!("notes_sync:{machine_id}")],
        )?;
        tx.commit()
    })?;
    Ok(notes(app)?
        .into_iter()
        .filter(|n| n["machine_id"] == json!(machine_id))
        .collect())
}

/// Every machine's project notes.
pub fn notes(app: &AppRef) -> rusqlite::Result<Vec<Value>> {
    app.db.with(|c| {
        let mut st = c.prepare(
            "select machine_id, path, text, updated_at from project_notes order by machine_id, path",
        )?;
        let rows = st.query_map([], |r| {
            Ok(json!({
                "machine_id": r.get::<_, i64>(0)?,
                "path": r.get::<_, String>(1)?,
                "text": r.get::<_, String>(2)?,
                "updated_at": r.get::<_, Option<i64>>(3)?,
            }))
        })?;
        rows.collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_follow_status_and_approval_changes() {
        assert_eq!(
            transitions(None, "running", None, false),
            vec!["session.started"]
        );
        assert!(transitions(None, "running", None, true).is_empty());
        assert_eq!(
            transitions(
                Some(("running", None)),
                "needs_you",
                Some("permission|Run?|ls"),
                false
            ),
            vec!["session.needs_you", "approval.requested"]
        );
        // Same approval again: nothing new.
        assert!(
            transitions(
                Some(("needs_you", Some("permission|Run?|ls"))),
                "needs_you",
                Some("permission|Run?|ls"),
                false
            )
            .is_empty()
        );
        // A different approval while still waiting.
        assert_eq!(
            transitions(
                Some(("needs_you", Some("a"))),
                "needs_you",
                Some("b"),
                false
            ),
            vec!["approval.requested"]
        );
        assert_eq!(
            transitions(Some(("running", None)), "done", None, false),
            vec!["session.done"]
        );
        assert_eq!(
            transitions(Some(("running", None)), "error", None, false),
            vec!["session.error"]
        );
    }

    #[test]
    fn views_are_checked_and_bounded() {
        let raw = json!({
            "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479", "project": "p", "title": "x".repeat(400),
            "agent": "claude", "status": "running",
            "preview": (0..20).map(|i| Some(format!("line {i}"))).chain([None]).collect::<Vec<_>>(),
            "approval": { "kind": "evil", "question": "Run?", "options": [{ "choice": "rm -rf", "label": "Yes" }] }
        });
        let (_, v) = serde_json::from_value::<SessionIn>(raw)
            .unwrap()
            .clean()
            .unwrap();
        assert_eq!(v["title"].as_str().unwrap().chars().count(), 300);
        assert_eq!(v["preview"].as_array().unwrap().len(), 12);
        assert_eq!(v["preview"][11], "");
        assert_eq!(v["approval"]["kind"], "permission");
        assert!(v["approval"]["options"][0]["choice"].is_null());

        let bad = json!({ "id": "not-a-uuid", "project": "p", "title": "t", "agent": "a", "status": "running" });
        assert!(
            serde_json::from_value::<SessionIn>(bad)
                .unwrap()
                .clean()
                .is_none()
        );
        let bad = json!({ "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479", "project": "p", "title": "t", "agent": "a", "status": "hacked" });
        assert!(
            serde_json::from_value::<SessionIn>(bad)
                .unwrap()
                .clean()
                .is_none()
        );
    }
}
