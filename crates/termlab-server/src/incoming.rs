//! Incoming webhooks: a secret URL per hook that starts a session when something calls it
//! (a CI job, GitHub, a form, a phone shortcut). Each hook fixes the computer, project,
//! agent and permissions; the request fills in the first message and the title through
//! templates like `{{body.issue.title}}`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::AppRef;
use crate::auth::{Client, Owner};
use crate::notify;
use crate::sessions;
use crate::util::{ApiError, ApiResult, clip, now, token};

/// Calls per hook per minute; more get 429.
const PER_MINUTE: usize = 20;
/// Calls kept per hook.
const CALLS_KEPT: i64 = 100;
/// A call the computer hasn't answered after this long shows as unanswered.
const ANSWER_WITHIN: Duration = Duration::from_secs(120);
const MESSAGE_MAX: usize = 20_000;
const TITLE_MAX: usize = 200;

pub fn routes() -> Router<AppRef> {
    Router::new()
        .route("/api/hooks/{token}", post(hook))
        .route("/api/incoming", get(list).post(create))
        .route("/api/incoming/{id}", patch(update).delete(remove))
        .route("/api/incoming/{id}/url", post(reveal))
        .route("/api/incoming/{id}/roll", post(roll))
        .route("/api/incoming/{id}/calls", get(calls))
        .route("/api/machines/{id}/labels", post(create_label))
}

/// The public URL is the only credential, so it's never answered without a valid one.
pub fn csrf_exempt(path: &str) -> bool {
    path.starts_with("/api/hooks/")
}

/// Browsers waiting for a computer to make a label, by request id.
#[derive(Default)]
pub struct LabelWaits(Mutex<HashMap<String, tokio::sync::oneshot::Sender<Value>>>);

/// A computer answered `label.create`: hand the answer to whoever asked.
pub fn label_created(app: &AppRef, machine: i64, msg: &Value) {
    if msg["labels"].is_array() {
        set_labels(app, machine, &msg["labels"]);
    }
    let Some(rid) = msg["request_id"].as_str() else {
        return;
    };
    let waiter = app
        .label_waits
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(rid);
    if let Some(tx) = waiter {
        let _ = tx.send(msg.clone());
    }
}

const LABEL_COLORS: [&str; 9] = [
    "red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "gray",
];

#[derive(Deserialize)]
struct LabelIn {
    name: String,
    color: String,
}

/// `POST /api/machines/{id}/labels`: makes a label on the computer (it keeps them), or
/// answers with the one of that name it already has. `{ label, labels }`.
async fn create_label(
    State(app): State<AppRef>,
    owner: Owner,
    Path(machine_id): Path<i64>,
    Json(b): Json<LabelIn>,
) -> ApiResult<Json<Value>> {
    let name = b.name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > LABEL_NAME_MAX {
        return Err(ApiError::bad(format!(
            "A label needs a name of at most {LABEL_NAME_MAX} characters."
        )));
    }
    if !LABEL_COLORS.contains(&b.color.as_str()) {
        return Err(ApiError::bad("Pick one of the label colours."));
    }
    let machine: String = app
        .db
        .with(|c| {
            c.query_row(
                "select name from machines where id = ?1",
                [machine_id],
                |r| r.get(0),
            )
            .optional()
        })?
        .ok_or_else(ApiError::not_found)?;
    let known = machine_labels(&app, machine_id);
    let lower = name.to_lowercase();
    if let Some(l) = known.as_array().and_then(|l| {
        l.iter().find(|l| {
            l["name"]
                .as_str()
                .is_some_and(|n| n.to_lowercase() == lower)
        })
    }) {
        return Ok(Json(json!({ "label": l, "labels": known })));
    }
    let request_id = format!("label-{}", uuid::Uuid::new_v4());
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.label_waits
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(request_id.clone(), tx);
    let forget = || {
        app.label_waits
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&request_id);
    };
    let sent = app.hub.to_machine(
        machine_id,
        &json!({ "type": "label.create", "request_id": request_id, "name": name, "color": b.color }),
    );
    if !sent {
        forget();
        return Err(ApiError::conflict(format!(
            "{machine} is offline. Labels are made in Termlab there."
        )));
    }
    let answer = match tokio::time::timeout(Duration::from_secs(8), rx).await {
        Ok(Ok(v)) => v,
        _ => {
            forget();
            return Err(ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "{machine} didn't answer. Making labels from here needs Termlab 0.2.14 there."
                ),
            ));
        }
    };
    if let Some(e) = answer["error"].as_str() {
        return Err(ApiError::bad(e.to_string()));
    }
    notify::audit(
        &app,
        "label.created",
        Some(&name),
        Some(&machine),
        None,
        &owner.ip,
    );
    Ok(Json(
        json!({ "label": answer["label"], "labels": machine_labels(&app, machine_id) }),
    ))
}

#[derive(Default)]
pub struct Limiter(Mutex<HashMap<i64, Vec<Instant>>>);

impl Limiter {
    fn allow(&self, hook: i64) -> bool {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let calls = map.entry(hook).or_default();
        calls.retain(|t| t.elapsed() < Duration::from_secs(60));
        if calls.len() >= PER_MINUTE {
            return false;
        }
        calls.push(Instant::now());
        true
    }
}

fn hook_url(app: &AppRef, token: &str) -> String {
    format!(
        "{}/api/hooks/{token}",
        app.cfg.public_url.trim_end_matches('/')
    )
}

fn new_token() -> String {
    format!("th_{}", token(24))
}

// ---- templates -------------------------------------------------------------------------

/// What a request offers to templates.
pub struct Request {
    pub body: String,
    /// The body as JSON, or as an object for a form post; null otherwise.
    pub json: Value,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
}

impl Request {
    pub fn new(
        body: &[u8],
        content_type: &str,
        query: HashMap<String, String>,
        headers: &HeaderMap,
    ) -> Self {
        let body = String::from_utf8_lossy(body).into_owned();
        let json = if content_type.starts_with("application/x-www-form-urlencoded") {
            Value::Object(
                url::form_urlencoded::parse(body.as_bytes())
                    .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                    .collect::<Map<_, _>>(),
            )
        } else {
            serde_json::from_str(&body).unwrap_or(Value::Null)
        };
        let headers = headers
            .iter()
            .filter_map(|(k, v)| {
                Some((
                    k.as_str().to_ascii_lowercase(),
                    v.to_str().ok()?.to_string(),
                ))
            })
            .collect();
        Self {
            body,
            json,
            query,
            headers,
        }
    }

    fn value(&self, key: &str) -> Option<String> {
        let text = |v: &Value| match v {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if key == "body" {
            return Some(self.body.clone());
        }
        if let Some(path) = key.strip_prefix("body.") {
            let found = path.split('.').try_fold(&self.json, |v, part| match v {
                Value::Object(m) => m.get(part),
                Value::Array(a) => a.get(part.parse::<usize>().ok()?),
                _ => None,
            });
            return Some(found.map(text).unwrap_or_default());
        }
        if let Some(name) = key.strip_prefix("query.") {
            return Some(self.query.get(name).cloned().unwrap_or_default());
        }
        if let Some(name) = key.strip_prefix("headers.") {
            return Some(
                self.headers
                    .get(&name.to_ascii_lowercase())
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        None
    }
}

/// Fills `{{body}}`, `{{body.a.b}}` (JSON or form fields; `.0` for list items),
/// `{{query.x}}` and `{{headers.x}}`. Missing values become empty; anything else in
/// braces stays as written.
pub fn render(template: &str, req: &Request) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = after[..end].trim();
        match req.value(key) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

// ---- the hook itself -------------------------------------------------------------------

struct Hook {
    id: i64,
    name: String,
    machine_id: i64,
    folder_id: i64,
    folder_path: String,
    agent: String,
    permission: String,
    model: Option<String>,
    title: String,
    message: String,
    notify: bool,
    enabled: bool,
    /// Picks the project from the request (blank: always the fixed one).
    project_template: String,
    /// Labels every session gets (the machine's label ids).
    label_ids: Vec<u32>,
    /// Label names from the request: a JSON list, or separated by commas.
    labels_template: String,
    /// Make labels the request names that don't exist yet.
    create_labels: bool,
}

const HOOK_COLUMNS: &str = "id, name, machine_id, folder_id, folder_path, agent, permission, model, title, message, notify, enabled, project_template, label_ids, labels_template, create_labels";
/// Where the columns after HOOK_COLUMNS start.
const AFTER: usize = 16;
/// Label names a call can add, and how long each can be (as in Termlab).
const LABELS_MAX: usize = 20;
const LABEL_NAME_MAX: usize = 32;

fn hook_row(r: &rusqlite::Row) -> rusqlite::Result<Hook> {
    Ok(Hook {
        id: r.get(0)?,
        name: r.get(1)?,
        machine_id: r.get(2)?,
        folder_id: r.get(3)?,
        folder_path: r.get(4)?,
        agent: r.get(5)?,
        permission: r.get(6)?,
        model: r.get(7)?,
        title: r.get(8)?,
        message: r.get(9)?,
        notify: r.get::<_, i64>(10)? != 0,
        enabled: r.get::<_, i64>(11)? != 0,
        project_template: r.get(12)?,
        label_ids: serde_json::from_str(&r.get::<_, String>(13)?).unwrap_or_default(),
        labels_template: r.get(14)?,
        create_labels: r.get::<_, i64>(15)? != 0,
    })
}

fn record_call(
    app: &AppRef,
    hook: i64,
    request_id: &str,
    ip: &str,
    status: &str,
    error: Option<&str>,
) {
    let _ = app.db.with(|c| {
        let t = now();
        c.execute(
            "insert into webhook_trigger_calls (trigger_id, request_id, ip, status, error, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6)",
            (hook, request_id, ip, status, error.map(|e| clip(e, 500)), &t),
        )?;
        c.execute("update webhook_triggers set last_call_at = ?1 where id = ?2", (&t, hook))?;
        c.execute(
            "delete from webhook_trigger_calls where trigger_id = ?1 and id not in
               (select id from webhook_trigger_calls where trigger_id = ?1 order by id desc limit ?2)",
            (hook, CALLS_KEPT),
        )
    });
}

/// The hook's fixed project (or fallback), by its folder (ids change when a folder is
/// added again). None when it has none, or the folder is gone.
fn find_project(app: &AppRef, hook: &Hook) -> Option<Value> {
    if hook.folder_path.is_empty() {
        return None;
    }
    let known = sessions::machine_projects(app, hook.machine_id);
    let list = known["projects"].as_array()?;
    list.iter()
        .find(|p| p["path"].as_str() == Some(hook.folder_path.as_str()))
        .or_else(|| {
            list.iter()
                .find(|p| p["id"].as_i64() == Some(hook.folder_id))
        })
        .cloned()
}

/// A project the request names: its name or folder name (any capitals), or its full path.
pub fn match_project(projects: &Value, wanted: &str) -> Option<Value> {
    let wanted = wanted.trim();
    let lower = wanted.to_lowercase();
    let list = projects.as_array()?;
    let folder = |p: &Value| {
        p["path"]
            .as_str()
            .unwrap_or_default()
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_lowercase()
    };
    list.iter()
        .find(|p| p["path"].as_str() == Some(wanted))
        .or_else(|| {
            list.iter().find(|p| {
                p["name"]
                    .as_str()
                    .is_some_and(|n| n.to_lowercase() == lower)
            })
        })
        .or_else(|| list.iter().find(|p| folder(p) == lower))
        .cloned()
}

/// Label names from a filled-in template: a JSON list (`["bug", "api"]`), or names
/// separated by commas or lines. Trimmed, once each (any capitals), at most 20.
pub fn label_names(text: &str) -> Vec<String> {
    let text = text.trim();
    let raw: Vec<String> = match serde_json::from_str::<Vec<Value>>(text) {
        Ok(list) if text.starts_with('[') => list
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                Value::Object(o) => o
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                other => other.to_string(),
            })
            .collect(),
        _ => text.split([',', '\n']).map(str::to_string).collect(),
    };
    let mut out: Vec<String> = Vec::new();
    for name in raw {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || out.iter().any(|o| o.to_lowercase() == name.to_lowercase()) {
            continue;
        }
        out.push(name.chars().take(LABEL_NAME_MAX).collect());
        if out.len() == LABELS_MAX {
            break;
        }
    }
    out
}

/// A computer's labels, as its Termlab reported them (`{ id, name, color }`).
pub fn set_labels(app: &AppRef, machine: i64, labels: &Value) {
    let list: Vec<Value> = labels
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|x| {
                    Some(json!({
                        "id": x["id"].as_u64()?,
                        "name": clip(x["name"].as_str()?, 100),
                        "color": clip(x["color"].as_str().unwrap_or("gray"), 20),
                    }))
                })
                .take(500)
                .collect()
        })
        .unwrap_or_default();
    let _ = app.db.set_meta(
        &format!("labels:{machine}"),
        &Value::Array(list).to_string(),
    );
}

fn machine_labels(app: &AppRef, machine: i64) -> Value {
    app.db
        .meta(&format!("labels:{machine}"))
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_else(|| json!([]))
}

fn agent_installed(app: &AppRef, machine_id: i64, agent: &str) -> Option<Value> {
    sessions::machine_projects(app, machine_id)["agents"]
        .as_array()?
        .iter()
        .find(|a| a["agent"].as_str() == Some(agent))
        .cloned()
}

/// `POST /api/hooks/{token}`: starts the hook's session.
async fn hook(
    State(app): State<AppRef>,
    client: Client,
    Path(tok): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let found = if tok.len() > 100 {
        None
    } else {
        app.db.with(|c| {
            c.query_row(
                &format!("select {HOOK_COLUMNS} from webhook_triggers where token = ?1"),
                [&tok],
                hook_row,
            )
            .optional()
        })?
    };
    let Some(hook) = found else {
        return Err(ApiError::not_found());
    };
    if !hook.enabled {
        record_call(&app, hook.id, "", &client.ip, "disabled", None);
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This webhook is turned off.",
        ));
    }
    if !app.hook_limiter.allow(hook.id) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many calls. Wait a minute and try again.",
        ));
    }

    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let req = Request::new(&body, &content_type, query, &headers);
    let message = clip(render(&hook.message, &req).trim(), MESSAGE_MAX);
    let title = clip(
        render(&hook.title, &req)
            .trim()
            .replace(['\n', '\r'], " ")
            .as_str(),
        TITLE_MAX,
    );
    let request_id = format!("hook-{}", uuid::Uuid::new_v4());

    let fail = |status: &str, error: String, code: StatusCode| {
        record_call(&app, hook.id, &request_id, &client.ip, status, Some(&error));
        Err(ApiError::new(code, error))
    };
    let machine: Option<String> = app.db.with(|c| {
        c.query_row(
            "select name from machines where id = ?1",
            [hook.machine_id],
            |r| r.get(0),
        )
        .optional()
    })?;
    let machine = machine.unwrap_or_default();
    let project = if hook.project_template.is_empty() {
        find_project(&app, &hook)
            .ok_or_else(|| "The project isn't on the computer any more.".to_string())
    } else {
        let wanted = render(&hook.project_template, &req);
        let wanted = wanted.trim();
        let named = (!wanted.is_empty())
            .then(|| {
                match_project(
                    &sessions::machine_projects(&app, hook.machine_id)["projects"],
                    wanted,
                )
            })
            .flatten();
        named.or_else(|| find_project(&app, &hook)).ok_or_else(|| {
            if wanted.is_empty() {
                "The request didn't name a project.".to_string()
            } else {
                format!("There's no project “{}” on {machine}.", clip(wanted, 100))
            }
        })
    };
    let project = match project {
        Ok(p) => p,
        Err(e) => return fail("failed", e, StatusCode::UNPROCESSABLE_ENTITY),
    };
    let Some(agent) = agent_installed(&app, hook.machine_id, &hook.agent) else {
        return fail(
            "failed",
            "The agent isn't installed on the computer any more.".into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    };
    let names = if hook.labels_template.is_empty() {
        Vec::new()
    } else {
        label_names(&render(&hook.labels_template, &req))
    };
    if !app.hub.machine_online(hook.machine_id) {
        return fail(
            "offline",
            format!("{machine} is offline."),
            StatusCode::CONFLICT,
        );
    }

    let detail = format!(
        "{} in {} ({}) via webhook “{}”",
        agent["name"].as_str().unwrap_or(&hook.agent),
        project["name"].as_str().unwrap_or_default(),
        hook.permission,
        hook.name,
    );
    notify::audit(
        &app,
        "session.start",
        Some(&detail),
        Some(&machine),
        None,
        &client.ip,
    );
    let sent = app.hub.to_machine(
        hook.machine_id,
        &json!({
            "type": "session.start", "request_id": request_id,
            "folder_id": project["id"].as_i64().unwrap_or(hook.folder_id),
            "agent": hook.agent, "permission": hook.permission, "model": hook.model,
            "message": Some(message).filter(|m| !m.is_empty()),
            "title": Some(title).filter(|t| !t.is_empty()),
            "notify": hook.notify,
            "labels": hook.label_ids, "label_names": names, "create_labels": hook.create_labels,
        }),
    );
    if !sent {
        return fail(
            "offline",
            format!("{machine} is offline."),
            StatusCode::CONFLICT,
        );
    }
    record_call(&app, hook.id, &request_id, &client.ip, "pending", None);
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "request_id": request_id })),
    )
        .into_response())
}

/// A computer answered a `session.start`; if a hook asked for it, note how it went.
pub fn start_result(app: &AppRef, request_id: &str, session: Option<&str>, error: Option<&str>) {
    if !request_id.starts_with("hook-") {
        return;
    }
    let status = if session.is_some() {
        "started"
    } else {
        "failed"
    };
    let _ = app.db.with(|c| {
        c.execute(
            "update webhook_trigger_calls set status = ?1, session_id = ?2, error = ?3
             where request_id = ?4 and status = 'pending'",
            (status, session, error.map(|e| clip(e, 500)), request_id),
        )
    });
}

// ---- managing hooks (owner) ------------------------------------------------------------

fn view(app: &AppRef, id: i64) -> ApiResult<Value> {
    let row = app.db.with(|c| {
        c.query_row(
            &format!(
                "select {HOOK_COLUMNS}, token, last_call_at, (select name from machines m where m.id = machine_id)
                 from webhook_triggers where id = ?1"
            ),
            [id],
            |r| Ok((hook_row(r)?, r.get::<_, String>(AFTER)?, r.get::<_, Option<String>>(AFTER + 1)?, r.get::<_, Option<String>>(AFTER + 2)?)),
        )
        .optional()
    })?;
    let (h, tok, last_call_at, machine) = row.ok_or_else(ApiError::not_found)?;
    let project = find_project(app, &h);
    let fixed = !h.folder_path.is_empty();
    let project_name = project
        .as_ref()
        .and_then(|p| p["name"].as_str().map(str::to_string))
        .or_else(|| {
            fixed.then(|| {
                h.folder_path
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
        });
    let url = hook_url(app, &tok);
    Ok(json!({
        "id": h.id,
        "name": h.name,
        "machine_id": h.machine_id,
        "machine": machine,
        // The fixed project, or with a project template the one used when the request's
        // doesn't match (null: the call fails).
        "folder_id": fixed.then(|| project.as_ref().and_then(|p| p["id"].as_i64()).unwrap_or(h.folder_id)),
        "folder_path": Some(&h.folder_path).filter(|_| fixed),
        "project": project_name,
        "project_missing": fixed && project.is_none(),
        "project_template": h.project_template,
        "label_ids": h.label_ids,
        "labels_template": h.labels_template,
        "create_labels": h.create_labels,
        "agent": h.agent,
        "permission": h.permission,
        "model": h.model,
        "title": h.title,
        "message": h.message,
        "notify": h.notify,
        "enabled": h.enabled,
        "last_call_at": last_call_at,
        // The full URL is a secret: shown on request only.
        "url_hint": format!("{}…", &url[..url.len() - tok.len() + 6.min(tok.len())]),
    }))
}

async fn list(State(app): State<AppRef>, _o: Owner) -> ApiResult<Json<Value>> {
    let ids: Vec<i64> = app.db.with(|c| {
        let mut st = c.prepare("select id from webhook_triggers order by id desc")?;
        st.query_map([], |r| r.get(0))?.collect()
    })?;
    let hooks = ids
        .into_iter()
        .map(|id| view(&app, id))
        .collect::<ApiResult<Vec<_>>>()?;
    let machines: Vec<i64> = app.db.with(|c| {
        let mut st = c.prepare("select id from machines")?;
        st.query_map([], |r| r.get(0))?.collect()
    })?;
    // Each computer's labels, for picking the ones a hook puts on its sessions.
    let labels: Map<String, Value> = machines
        .into_iter()
        .map(|m| (m.to_string(), machine_labels(&app, m)))
        .collect();
    Ok(Json(json!({ "hooks": hooks, "labels": labels })))
}

#[derive(Deserialize)]
struct HookIn {
    name: Option<String>,
    machine_id: Option<i64>,
    folder_id: Option<i64>,
    agent: Option<String>,
    permission: Option<String>,
    model: Option<String>,
    title: Option<String>,
    message: Option<String>,
    notify: Option<bool>,
    enabled: Option<bool>,
    project_template: Option<String>,
    label_ids: Option<Vec<u32>>,
    labels_template: Option<String>,
    create_labels: Option<bool>,
}

/// A complete, checked hook. `current` fills what the request leaves out.
struct Checked {
    name: String,
    machine_id: i64,
    folder_id: i64,
    folder_path: String,
    agent: String,
    permission: String,
    model: Option<String>,
    title: String,
    message: String,
    notify: bool,
    project_template: String,
    label_ids: Vec<u32>,
    labels_template: String,
    create_labels: bool,
}

fn check(app: &AppRef, b: HookIn, current: Option<&Hook>) -> ApiResult<Checked> {
    let name = b
        .name
        .or_else(|| current.map(|h| h.name.clone()))
        .map(|n| clip(n.trim(), 100))
        .filter(|n| !n.is_empty())
        .ok_or_else(|| ApiError::bad("Give the webhook a name."))?;
    let machine_id = b
        .machine_id
        .or(current.map(|h| h.machine_id))
        .ok_or_else(|| ApiError::bad("Pick a computer."))?;
    let exists: i64 = app.db.with(|c| {
        c.query_row(
            "select count(*) from machines where id = ?1",
            [machine_id],
            |r| r.get(0),
        )
    })?;
    if exists == 0 {
        return Err(ApiError::bad("Pick a computer."));
    }
    let known = sessions::machine_projects(app, machine_id);
    let same_machine = current.is_some_and(|h| h.machine_id == machine_id);
    let project_template = b
        .project_template
        .or_else(|| current.map(|h| h.project_template.clone()))
        .unwrap_or_default()
        .trim()
        .to_string();
    // 0: no fixed project (only with a project template).
    let (folder_id, folder_path) = match (b.folder_id, current) {
        (Some(0), _) => (0, String::new()),
        (Some(f), _) => {
            let p = known["projects"]
                .as_array()
                .and_then(|l| l.iter().find(|p| p["id"].as_i64() == Some(f)))
                .ok_or_else(|| ApiError::bad("That project isn't on this computer."))?;
            (f, p["path"].as_str().unwrap_or_default().to_string())
        }
        (None, Some(h)) if same_machine => (h.folder_id, h.folder_path.clone()),
        _ => (0, String::new()),
    };
    if project_template.is_empty() && folder_path.is_empty() {
        return Err(ApiError::bad("Pick a project."));
    }
    let label_ids = b
        .label_ids
        .or_else(|| {
            current
                .filter(|_| same_machine)
                .map(|h| h.label_ids.clone())
        })
        .unwrap_or_default();
    let labels_template = b
        .labels_template
        .or_else(|| current.map(|h| h.labels_template.clone()))
        .unwrap_or_default()
        .trim()
        .to_string();
    if label_ids.len() > 50
        || project_template.chars().count() > 200
        || labels_template.chars().count() > 500
    {
        return Err(ApiError::bad(
            "Too many labels, or a template that's too long.",
        ));
    }
    let agent = b
        .agent
        .or_else(|| current.map(|h| h.agent.clone()))
        .ok_or_else(|| ApiError::bad("Pick an agent."))?;
    if agent_installed(app, machine_id, &agent).is_none() {
        return Err(ApiError::bad(
            "That agent isn't installed on this computer.",
        ));
    }
    let permission = b
        .permission
        .or_else(|| current.map(|h| h.permission.clone()))
        .unwrap_or_else(|| "ask_first".into());
    if !matches!(permission.as_str(), "ask_first" | "auto_edit" | "full_auto") {
        return Err(ApiError::bad("Unknown permission setting."));
    }
    let model = match b.model {
        Some(m) => Some(clip(m.trim(), 100)).filter(|m| !m.is_empty()),
        None => current.and_then(|h| h.model.clone()),
    };
    let title = b
        .title
        .or_else(|| current.map(|h| h.title.clone()))
        .unwrap_or_default();
    let message = b
        .message
        .or_else(|| current.map(|h| h.message.clone()))
        .unwrap_or_else(|| "{{body}}".into());
    if title.chars().count() > 500 || message.chars().count() > MESSAGE_MAX {
        return Err(ApiError::bad("The title or message template is too long."));
    }
    Ok(Checked {
        name,
        machine_id,
        folder_id,
        folder_path,
        agent,
        permission,
        model,
        title: title.trim().to_string(),
        message,
        notify: b.notify.or(current.map(|h| h.notify)).unwrap_or(true),
        project_template,
        label_ids,
        labels_template,
        create_labels: b
            .create_labels
            .or(current.map(|h| h.create_labels))
            .unwrap_or(false),
    })
}

async fn create(
    State(app): State<AppRef>,
    owner: Owner,
    Json(b): Json<HookIn>,
) -> ApiResult<Response> {
    let enabled = b.enabled.unwrap_or(true);
    let h = check(&app, b, None)?;
    let id = app.db.with(|c| {
        c.execute(
            "insert into webhook_triggers (name, token, machine_id, folder_id, folder_path, agent, permission,
               model, title, message, notify, enabled, created_at, project_template, label_ids, labels_template,
               create_labels)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            rusqlite::params![
                h.name, new_token(), h.machine_id, h.folder_id, h.folder_path, h.agent, h.permission,
                h.model, h.title, h.message, h.notify, enabled, now(), h.project_template,
                serde_json::to_string(&h.label_ids).unwrap_or_else(|_| "[]".into()), h.labels_template,
                h.create_labels
            ],
        )?;
        Ok(c.last_insert_rowid())
    })?;
    notify::audit(
        &app,
        "incoming.created",
        Some(&h.name),
        None,
        None,
        &owner.ip,
    );
    Ok((StatusCode::CREATED, Json(view(&app, id)?)).into_response())
}

fn load(app: &AppRef, id: i64) -> ApiResult<Hook> {
    app.db
        .with(|c| {
            c.query_row(
                &format!("select {HOOK_COLUMNS} from webhook_triggers where id = ?1"),
                [id],
                hook_row,
            )
            .optional()
        })?
        .ok_or_else(ApiError::not_found)
}

async fn update(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<i64>,
    Json(b): Json<HookIn>,
) -> ApiResult<Json<Value>> {
    let current = load(&app, id)?;
    let enabled = b.enabled;
    let only_toggle = b.name.is_none()
        && b.machine_id.is_none()
        && b.folder_id.is_none()
        && b.agent.is_none()
        && b.permission.is_none()
        && b.model.is_none()
        && b.title.is_none()
        && b.message.is_none()
        && b.notify.is_none()
        && b.project_template.is_none()
        && b.label_ids.is_none()
        && b.labels_template.is_none()
        && b.create_labels.is_none();
    if !only_toggle {
        let h = check(&app, b, Some(&current))?;
        app.db.with(|c| {
            c.execute(
                "update webhook_triggers set name = ?1, machine_id = ?2, folder_id = ?3, folder_path = ?4,
                   agent = ?5, permission = ?6, model = ?7, title = ?8, message = ?9, notify = ?10,
                   project_template = ?11, label_ids = ?12, labels_template = ?13, create_labels = ?14
                 where id = ?15",
                rusqlite::params![
                    h.name, h.machine_id, h.folder_id, h.folder_path, h.agent, h.permission, h.model,
                    h.title, h.message, h.notify, h.project_template,
                    serde_json::to_string(&h.label_ids).unwrap_or_else(|_| "[]".into()),
                    h.labels_template, h.create_labels, id
                ],
            )
        })?;
        notify::audit(
            &app,
            "incoming.updated",
            Some(&h.name),
            None,
            None,
            &owner.ip,
        );
    }
    if let Some(on) = enabled {
        app.db.with(|c| {
            c.execute(
                "update webhook_triggers set enabled = ?1 where id = ?2",
                (on, id),
            )
        })?;
    }
    Ok(Json(view(&app, id)?))
}

async fn remove(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let h = load(&app, id)?;
    app.db
        .with(|c| c.execute("delete from webhook_triggers where id = ?1", [id]))?;
    notify::audit(
        &app,
        "incoming.deleted",
        Some(&h.name),
        None,
        None,
        &owner.ip,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn reveal(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let tok: Option<String> = app.db.with(|c| {
        c.query_row(
            "select token from webhook_triggers where id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()
    })?;
    let tok = tok.ok_or_else(ApiError::not_found)?;
    Ok(Json(json!({ "url": hook_url(&app, &tok) })))
}

async fn roll(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let h = load(&app, id)?;
    let tok = new_token();
    app.db.with(|c| {
        c.execute(
            "update webhook_triggers set token = ?1 where id = ?2",
            (&tok, id),
        )
    })?;
    notify::audit(
        &app,
        "incoming.url_rolled",
        Some(&h.name),
        None,
        None,
        &owner.ip,
    );
    Ok(Json(json!({ "url": hook_url(&app, &tok) })))
}

async fn calls(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    load(&app, id)?;
    let cutoff = time::OffsetDateTime::now_utc() - ANSWER_WITHIN;
    let list: Vec<Value> = app.db.with(|c| {
        let mut st = c.prepare(
            "select id, ip, status, session_id, error, created_at from webhook_trigger_calls
             where trigger_id = ?1 order by id desc limit 50",
        )?;
        let rows = st.query_map([id], |r| {
            let created: String = r.get(5)?;
            let mut status: String = r.get(2)?;
            let stale = time::OffsetDateTime::parse(
                &created,
                &time::format_description::well_known::Rfc3339,
            )
            .is_ok_and(|t| t < cutoff);
            if status == "pending" && stale {
                status = "no_answer".into();
            }
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "ip": r.get::<_, Option<String>>(1)?,
                "status": status,
                "session_id": r.get::<_, Option<String>>(3)?,
                "error": r.get::<_, Option<String>>(4)?,
                "created_at": created,
            }))
        })?;
        rows.collect()
    })?;
    Ok(Json(json!(list)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(body: &str, ct: &str) -> Request {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-event", "issues".parse().unwrap());
        let query = HashMap::from([("branch".to_string(), "main".to_string())]);
        Request::new(body.as_bytes(), ct, query, &headers)
    }

    #[test]
    fn templates_fill_body_json_query_and_headers() {
        let r = req(
            r#"{"issue":{"title":"Fix login","labels":[{"name":"bug"}],"number":42}}"#,
            "application/json",
        );
        assert_eq!(
            render(
                "#{{ body.issue.number }} {{body.issue.title}} ({{body.issue.labels.0.name}})",
                &r
            ),
            "#42 Fix login (bug)"
        );
        assert_eq!(
            render("on {{query.branch}} from {{headers.X-GitHub-Event}}", &r),
            "on main from issues"
        );
        assert_eq!(render("{{body.issue.missing}}|{{query.none}}", &r), "|");
        assert_eq!(render("{{body.issue.labels}}", &r), r#"[{"name":"bug"}]"#);
    }

    #[test]
    fn templates_keep_unknown_keys_and_raw_bodies() {
        let r = req("just text", "text/plain");
        assert_eq!(render("Do: {{body}}", &r), "Do: just text");
        assert_eq!(render("{{body.x}}", &r), "");
        assert_eq!(render("{{other}} {{ unclosed", &r), "{{other}} {{ unclosed");
    }

    #[test]
    fn form_posts_are_fields() {
        let r = req(
            "text=deploy+staging&user_name=pat",
            "application/x-www-form-urlencoded",
        );
        assert_eq!(
            render("{{body.user_name}}: {{body.text}}", &r),
            "pat: deploy staging"
        );
    }

    #[test]
    fn requests_name_projects_by_name_folder_or_path() {
        let projects = json!([
            { "id": 1, "name": "API Server", "path": "/Users/me/Code/api" },
            { "id": 2, "name": "web", "path": "C:\\Code\\web-app\\" },
        ]);
        let id = |w: &str| match_project(&projects, w).and_then(|p| p["id"].as_i64());
        assert_eq!(id("api server"), Some(1));
        assert_eq!(id(" API "), Some(1));
        assert_eq!(id("/Users/me/Code/api"), Some(1));
        assert_eq!(id("Web-App"), Some(2));
        assert_eq!(id("WEB"), Some(2));
        assert_eq!(id("nope"), None);
    }

    #[test]
    fn label_names_come_as_lists_or_commas() {
        assert_eq!(
            label_names(r#"["bug", "API", "bug", {"name":"ci"}]"#),
            ["bug", "API", "ci"]
        );
        assert_eq!(
            label_names(" bug,  Client   work ,,BUG\nurgent "),
            ["bug", "Client work", "urgent"]
        );
        assert!(label_names("  ").is_empty());
        assert_eq!(label_names(&"x".repeat(40))[0].len(), LABEL_NAME_MAX);
        let many: Vec<String> = (0..30).map(|i| format!("l{i}")).collect();
        assert_eq!(label_names(&many.join(",")).len(), LABELS_MAX);
    }

    #[test]
    fn limiter_allows_twenty_a_minute_per_hook() {
        let l = Limiter::default();
        for _ in 0..PER_MINUTE {
            assert!(l.allow(1));
        }
        assert!(!l.allow(1));
        assert!(l.allow(2));
    }
}
