//! The browser API (see docs/server-protocol.md).

use axum::extract::{Path, Query, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{self, MIN_PASSWORD, Owner};
use crate::notify::{self, EVENTS, FAILING_AFTER, TEST_EVENT};
use crate::sessions;
use crate::util::{ApiError, ApiResult, clip, now, sha256_hex, token, unix_now};
use crate::{AppRef, SetupState};

pub fn routes() -> Router<AppRef> {
    Router::new()
        .route("/api/state", get(state))
        .route("/api/setup/start", post(setup_start))
        .route("/api/setup/finish", post(setup_finish))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/overview", get(overview))
        .route("/api/sessions/{id}/answer", post(answer))
        .route("/api/sessions/{id}/message", post(message))
        .route("/api/sessions/{id}/stop", post(stop))
        .route("/api/sessions/{id}/hold", post(hold))
        .route("/api/sessions/{id}/unhold", post(unhold))
        .route("/api/machines/{id}/sessions", post(start_session))
        .route("/api/machines/{id}/projects/move", post(move_project))
        .route("/api/machines", get(machines))
        .route("/api/pairing-codes", post(pairing_code))
        .route(
            "/api/machines/{id}",
            patch(rename_machine).delete(remove_machine),
        )
        .route("/api/machines/{id}/notes", axum::routing::put(save_notes))
        .route("/api/webhooks", get(webhooks).post(create_webhook))
        .route(
            "/api/webhooks/{id}",
            patch(update_webhook).delete(delete_webhook),
        )
        .route("/api/webhooks/{id}/secret", post(webhook_secret))
        .route("/api/webhooks/{id}/roll", post(roll_secret))
        .route("/api/webhooks/{id}/test", post(test_webhook))
        .route("/api/webhooks/{id}/deliveries", get(deliveries))
        .route("/api/deliveries/{id}/redeliver", post(redeliver))
        .route("/api/account/password", post(change_password))
        .route("/api/account/totp/start", post(totp_start))
        .route("/api/account/totp/finish", post(totp_finish))
        .route("/api/account/sessions", get(web_sessions))
        .route("/api/account/sessions/{id}", delete(end_web_session))
        .route("/api/audit", get(audit))
        .route("/api/push", get(push_list).post(push_add))
        .route("/api/push/test", post(push_test))
        .route("/api/push/{id}", delete(push_remove))
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn accepted(v: Value) -> Response {
    (StatusCode::ACCEPTED, Json(v)).into_response()
}

fn with_cookie(cookie: String) -> Response {
    let mut res = no_content();
    res.headers_mut()
        .insert(SET_COOKIE, cookie.parse().expect("ascii cookie"));
    res
}

/// Compares secrets without leaking where they differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

// ---- setup and sign-in -----------------------------------------------------------------

async fn state(State(app): State<AppRef>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let signed_in = match auth::cookie_value(&headers) {
        Some(raw) => {
            app.db.with(|c| {
                c.query_row(
                    "select count(*) from web_sessions where token_hash = ?1",
                    [sha256_hex(&raw)],
                    |r| r.get::<_, i64>(0),
                )
            })? > 0
        }
        None => false,
    };
    Ok(Json(json!({
        "setup_required": !app.db.has_owner(),
        "signed_in": signed_in,
        "version": env!("CARGO_PKG_VERSION"),
        "server_name": app.cfg.server_name,
        "vapid_public_key": notify::vapid_public(&app.vapid),
    })))
}

#[derive(Deserialize)]
struct SetupStart {
    setup_token: String,
}

fn check_setup_token(app: &AppRef, given: &str) -> Result<(), ApiError> {
    if app.db.has_owner() {
        return Err(ApiError::conflict(
            "This server already has an owner. Sign in instead.",
        ));
    }
    let guard = app.setup.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(s) if same(&s.token, given.trim()) => Ok(()),
        _ => Err(ApiError::unauthorized(
            "That setup token isn't right. It's printed in the server's log.",
        )),
    }
}

async fn setup_start(
    State(app): State<AppRef>,
    Json(b): Json<SetupStart>,
) -> ApiResult<Json<Value>> {
    check_setup_token(&app, &b.setup_token)?;
    let secret = auth::new_totp_secret();
    if let Some(s) = app.setup.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        s.totp_secret = Some(secret.clone());
    }
    Ok(Json(auth::enrolment(&secret, &app.cfg.server_name)))
}

#[derive(Deserialize)]
struct SetupFinish {
    setup_token: String,
    password: String,
    totp_code: String,
}

async fn setup_finish(
    State(app): State<AppRef>,
    client: auth::Client,
    Json(b): Json<SetupFinish>,
) -> ApiResult<Response> {
    check_setup_token(&app, &b.setup_token)?;
    if b.password.chars().count() < MIN_PASSWORD {
        return Err(ApiError::bad(format!(
            "Use a password of at least {MIN_PASSWORD} characters."
        )));
    }
    let secret = app
        .setup
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|s: &SetupState| s.totp_secret.clone())
        .ok_or_else(|| {
            ApiError::conflict("Start the setup again: the authenticator step is missing.")
        })?;
    if !auth::check_totp(&secret, &b.totp_code) {
        return Err(ApiError::unauthorized(
            "That code doesn't match. Check the time on your phone and try the next code.",
        ));
    }
    let hash = auth::hash_password(&b.password)
        .map_err(|_| ApiError::bad("Couldn't store the password."))?;
    let inserted = app.db.with(|c| {
        c.execute(
            "insert or ignore into owner (id, password_hash, totp_secret, created_at) values (1, ?1, ?2, ?3)",
            (&hash, &secret, now()),
        )
    })?;
    if inserted == 0 {
        return Err(ApiError::conflict(
            "This server already has an owner. Sign in instead.",
        ));
    }
    *app.setup.lock().unwrap_or_else(|e| e.into_inner()) = None;
    notify::audit(&app, "account.created", None, None, None, &client.ip);
    let raw = auth::start_session(&app, &client.user_agent, &client.ip)?;
    Ok(with_cookie(auth::set_cookie(&app, &raw, 30)))
}

#[derive(Deserialize)]
struct Login {
    password: String,
    totp_code: String,
}

async fn login(
    State(app): State<AppRef>,
    client: auth::Client,
    Json(b): Json<Login>,
) -> ApiResult<Response> {
    if let Err(wait) = app.limiter.check(&client.ip) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "Too many attempts. Try again in {} minutes.",
                wait.div_ceil(60)
            ),
        ));
    }
    let (hash, secret) = owner_row(&app)?;
    // Check both, always, so timing doesn't tell which one was wrong.
    let password_ok = auth::verify_password(&b.password, &hash);
    let code_ok = auth::check_totp(&secret, &b.totp_code);
    if !(password_ok && code_ok) {
        app.limiter.failed(&client.ip);
        notify::audit(&app, "account.sign_in_failed", None, None, None, &client.ip);
        return Err(ApiError::unauthorized(
            "The password or the code isn't right.",
        ));
    }
    app.limiter.succeeded(&client.ip);
    notify::audit(
        &app,
        "account.signed_in",
        Some(&clip(&client.user_agent, 200)),
        None,
        None,
        &client.ip,
    );
    let raw = auth::start_session(&app, &client.user_agent, &client.ip)?;
    Ok(with_cookie(auth::set_cookie(&app, &raw, 30)))
}

async fn logout(State(app): State<AppRef>, owner: Option<Owner>) -> ApiResult<Response> {
    if let Some(o) = owner {
        app.db
            .with(|c| c.execute("delete from web_sessions where id = ?1", [o.session_id]))?;
        app.hub.close_web_session(o.session_id);
    }
    Ok(with_cookie(auth::clear_cookie(&app)))
}

// ---- remote ----------------------------------------------------------------------------

async fn overview(State(app): State<AppRef>, _o: Owner) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({ "machines": sessions::machines(&app)?, "sessions": sessions::all(&app)?, "notes": sessions::notes(&app)?, "usage": sessions::usage(&app)?, "projects": sessions::projects(&app)? }),
    ))
}

#[derive(Deserialize)]
struct Answer {
    choice: String,
}

#[derive(Deserialize)]
struct Message {
    text: String,
}

/// Audits and sends a command to the session's machine.
fn command(
    app: &AppRef,
    owner: &Owner,
    id: &str,
    cmd: &str,
    data: Value,
    detail: Option<String>,
) -> ApiResult<Response> {
    let Some((view, machine_id, machine)) = sessions::find(app, id)? else {
        return Err(ApiError::not_found());
    };
    if !app.hub.machine_online(machine_id) {
        return Err(ApiError::conflict(format!("{machine} is offline.")));
    }
    let title = view["title"].as_str().unwrap_or_default().to_string();
    let action = match (cmd, data.get("choice").and_then(Value::as_str)) {
        ("answer", Some(choice)) => format!("approval.{choice}"),
        (c, _) => format!("session.{c}"),
    };
    notify::audit(
        app,
        &action,
        detail.as_deref(),
        Some(&machine),
        Some(&title),
        &owner.ip,
    );
    let command_id = uuid::Uuid::new_v4().to_string();
    let sent = app.hub.to_machine(
        machine_id,
        &json!({ "type": "command", "id": command_id, "command": cmd, "session_id": id, "data": data }),
    );
    if !sent {
        return Err(ApiError::conflict(format!("{machine} is offline.")));
    }
    Ok(accepted(json!({ "command_id": command_id })))
}

async fn answer(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<String>,
    Json(b): Json<Answer>,
) -> ApiResult<Response> {
    if !matches!(b.choice.as_str(), "allow_once" | "allow_always" | "deny") {
        return Err(ApiError::bad("Unknown choice."));
    }
    let Some((view, _, _)) = sessions::find(&app, &id)? else {
        return Err(ApiError::not_found());
    };
    if view["status"] != "needs_you" || view["approval"].is_null() {
        return Err(ApiError::conflict(
            "This session is no longer waiting for an approval.",
        ));
    }
    let detail = view["approval"]["detail"]
        .as_str()
        .or(view["approval"]["question"].as_str())
        .map(str::to_string);
    command(
        &app,
        &owner,
        &id,
        "answer",
        json!({ "choice": b.choice }),
        detail,
    )
}

async fn message(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<String>,
    Json(b): Json<Message>,
) -> ApiResult<Response> {
    if b.text.trim().is_empty() || b.text.len() > 10_000 {
        return Err(ApiError::bad("Type a message (up to 10,000 characters)."));
    }
    let detail = Some(format!("{} characters", b.text.chars().count()));
    command(
        &app,
        &owner,
        &id,
        "message",
        json!({ "text": b.text }),
        detail,
    )
}

async fn stop(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    command(&app, &owner, &id, "stop", json!({}), None)
}

async fn hold(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    command(&app, &owner, &id, "hold", json!({}), None)
}

async fn unhold(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    command(&app, &owner, &id, "unhold", json!({}), None)
}

#[derive(Deserialize)]
struct StartSession {
    request_id: String,
    folder_id: i64,
    agent: String,
    permission: String,
    model: Option<String>,
    message: Option<String>,
    notify: Option<bool>,
}

/// Starts a new session on a machine, in one of its projects, with an agent installed there.
async fn start_session(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<i64>,
    Json(b): Json<StartSession>,
) -> ApiResult<Response> {
    let known = sessions::machine_projects(&app, id);
    let project = known["projects"]
        .as_array()
        .and_then(|l| {
            l.iter()
                .find(|p| p["id"].as_i64() == Some(b.folder_id))
                .cloned()
        })
        .ok_or_else(|| ApiError::bad("That project isn't on this computer."))?;
    let agent = known["agents"]
        .as_array()
        .and_then(|l| {
            l.iter()
                .find(|a| a["agent"].as_str() == Some(b.agent.as_str()))
                .cloned()
        })
        .ok_or_else(|| ApiError::bad("That agent isn't installed on this computer."))?;
    if !matches!(
        b.permission.as_str(),
        "ask_first" | "auto_edit" | "full_auto"
    ) {
        return Err(ApiError::bad("Unknown permission setting."));
    }
    if b.request_id.len() > 64 || b.message.as_ref().is_some_and(|m| m.len() > 20_000) {
        return Err(ApiError::bad("The first message is too long."));
    }
    let machine: String = app
        .db
        .with(|c| {
            c.query_row("select name from machines where id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
        })?
        .ok_or_else(ApiError::not_found)?;
    if !app.hub.machine_online(id) {
        return Err(ApiError::conflict(format!("{machine} is offline.")));
    }
    let detail = format!(
        "{} in {} ({})",
        agent["name"].as_str().unwrap_or(&b.agent),
        project["name"].as_str().unwrap_or_default(),
        b.permission
    );
    notify::audit(
        &app,
        "session.start",
        Some(&detail),
        Some(&machine),
        None,
        &owner.ip,
    );
    let sent = app.hub.to_machine(
        id,
        &json!({
            "type": "session.start", "request_id": b.request_id, "folder_id": b.folder_id,
            "agent": b.agent, "permission": b.permission, "model": b.model, "message": b.message,
            "notify": b.notify.unwrap_or(true),
        }),
    );
    if !sent {
        return Err(ApiError::conflict(format!("{machine} is offline.")));
    }
    Ok(accepted(json!({})))
}

#[derive(Deserialize)]
struct MoveProject {
    folder_id: i64,
    /// The project it goes before; none puts it last.
    before: Option<i64>,
}

/// Reorders a machine's projects (Termlab's sidebar order, shared with the app).
async fn move_project(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
    Json(b): Json<MoveProject>,
) -> ApiResult<Response> {
    let known = sessions::machine_projects(&app, id);
    let has = |f: i64| {
        known["projects"]
            .as_array()
            .is_some_and(|l| l.iter().any(|p| p["id"].as_i64() == Some(f)))
    };
    if !has(b.folder_id) || b.before.is_some_and(|f| !has(f)) {
        return Err(ApiError::not_found());
    }
    let sent = app.hub.to_machine(
        id,
        &json!({ "type": "project.move", "folder_id": b.folder_id, "before": b.before }),
    );
    if !sent {
        return Err(ApiError::conflict("The computer is offline."));
    }
    Ok(accepted(json!({})))
}

// ---- machines --------------------------------------------------------------------------

async fn machines(State(app): State<AppRef>, _o: Owner) -> ApiResult<Json<Value>> {
    Ok(Json(json!(sessions::machines(&app)?)))
}

/// "K7QX-2MPD": no 0/O or 1/I, so it's easy to type.
pub fn new_pairing_code() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rngs::OsRng;
    let raw: String = (0..8)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect();
    format!("{}-{}", &raw[..4], &raw[4..])
}

pub fn normalize_code(code: &str) -> String {
    let raw: String = code
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_uppercase();
    if raw.len() == 8 {
        format!("{}-{}", &raw[..4], &raw[4..])
    } else {
        raw
    }
}

async fn pairing_code(State(app): State<AppRef>, owner: Owner) -> ApiResult<Json<Value>> {
    let code = new_pairing_code();
    let expires = unix_now() + 600;
    app.db.with(|c| {
        c.execute("delete from pairing_codes", [])?;
        c.execute(
            "insert into pairing_codes (code_hash, expires_at) values (?1, ?2)",
            (sha256_hex(&code), expires),
        )
    })?;
    notify::audit(&app, "machine.pairing_code", None, None, None, &owner.ip);
    let expires_at = time::OffsetDateTime::from_unix_timestamp(expires)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        });
    Ok(Json(json!({ "code": code, "expires_at": expires_at })))
}

#[derive(Deserialize)]
struct NotesSave {
    path: String,
    text: String,
}

/// Saves a project's notes on the machine, which writes them and sends them back.
async fn save_notes(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
    Json(b): Json<NotesSave>,
) -> ApiResult<Response> {
    if b.text.chars().count() > sessions::NOTES_MAX {
        return Err(ApiError::bad("Notes can be up to 100,000 characters."));
    }
    // Only a project folder the machine reported; it checks that again.
    let known: bool = app.db.with(|c| {
        c.query_row(
            "select exists(select 1 from sessions where machine_id = ?1 and json_extract(view, '$.directory') = ?2)
                 or exists(select 1 from project_notes where machine_id = ?1 and path = ?2)",
            (id, &b.path),
            |r| r.get(0),
        )
    })?;
    if !known {
        return Err(ApiError::not_found());
    }
    let sent = app.hub.to_machine(
        id,
        &json!({ "type": "notes.save", "path": b.path, "text": b.text }),
    );
    if !sent {
        return Err(ApiError::conflict(
            "The computer is offline: the notes can't be saved now.",
        ));
    }
    Ok(accepted(json!({})))
}

#[derive(Deserialize)]
struct Rename {
    name: String,
}

async fn rename_machine(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
    Json(b): Json<Rename>,
) -> ApiResult<Json<Value>> {
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad("Give it a name of up to 100 characters."));
    }
    let n = app
        .db
        .with(|c| c.execute("update machines set name = ?1 where id = ?2", (name, id)))?;
    if n == 0 {
        return Err(ApiError::not_found());
    }
    let m = sessions::machine_view(&app, id)?.ok_or_else(ApiError::not_found)?;
    app.hub
        .broadcast(&json!({ "type": "machine.updated", "machine": m }));
    Ok(Json(m))
}

async fn remove_machine(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let (name, session_ids): (String, Vec<String>) = app
        .db
        .with(|c| {
            let name: Option<String> = c
                .query_row("select name from machines where id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            let mut st = c.prepare("select id from sessions where machine_id = ?1")?;
            let ids = st
                .query_map([id], |r| r.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            Ok(name.map(|n| (n, ids)))
        })?
        .ok_or_else(ApiError::not_found)?;
    app.db.with(|c| {
        c.execute("delete from machines where id = ?1", [id])?;
        c.execute(
            "delete from meta where key in (?1, ?2, ?3)",
            [
                format!("notes_sync:{id}"),
                format!("usage:{id}"),
                format!("projects:{id}"),
            ],
        )
    })?;
    app.hub.revoke_machine(id);
    for sid in session_ids {
        app.hub
            .broadcast(&json!({ "type": "session.removed", "id": sid }));
    }
    notify::audit(&app, "machine.removed", None, Some(&name), None, &owner.ip);
    Ok(no_content())
}

// ---- webhooks --------------------------------------------------------------------------

fn endpoint_view(app: &AppRef, id: i64) -> ApiResult<Value> {
    let v = app.db.with(|c| {
        c.query_row(
            "select id, url, description, events, enabled, consecutive_failures, last_delivery_at from webhook_endpoints where id = ?1",
            [id],
            |r| {
                let enabled = r.get::<_, i64>(4)? != 0;
                let failures: i64 = r.get(5)?;
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "url": r.get::<_, String>(1)?,
                    "description": r.get::<_, Option<String>>(2)?,
                    "events": serde_json::from_str::<Value>(&r.get::<_, String>(3)?).unwrap_or(json!([])),
                    "enabled": enabled,
                    "health": if !enabled { "disabled" } else if failures >= FAILING_AFTER { "failing" } else { "healthy" },
                    "last_delivery_at": r.get::<_, Option<String>>(6)?,
                }))
            },
        )
        .optional()
    })?;
    v.ok_or_else(ApiError::not_found)
}

async fn webhooks(State(app): State<AppRef>, _o: Owner) -> ApiResult<Json<Value>> {
    let ids: Vec<i64> = app.db.with(|c| {
        let mut st = c.prepare("select id from webhook_endpoints order by id desc")?;
        st.query_map([], |r| r.get(0))?.collect()
    })?;
    let endpoints = ids
        .into_iter()
        .map(|id| endpoint_view(&app, id))
        .collect::<ApiResult<Vec<_>>>()?;
    let events: Vec<Value> = EVENTS
        .iter()
        .map(|(n, d)| json!({ "name": n, "description": d }))
        .collect();
    Ok(Json(json!({ "endpoints": endpoints, "events": events })))
}

#[derive(Deserialize)]
struct WebhookIn {
    url: Option<String>,
    description: Option<String>,
    events: Option<Vec<String>>,
    enabled: Option<bool>,
}

fn validate_url(app: &AppRef, url: &str) -> ApiResult<String> {
    let url = url.trim();
    let parsed = url::Url::parse(url)
        .map_err(|_| ApiError::bad("Enter a full URL, like https://hooks.example.com/termlab."))?;
    let ok =
        parsed.scheme() == "https" || (app.cfg.allow_private_webhooks && parsed.scheme() == "http");
    if !ok || parsed.host_str().is_none() || url.len() > 2048 {
        return Err(ApiError::bad(if app.cfg.allow_private_webhooks {
            "Enter a valid URL."
        } else {
            "Enter a valid https:// URL."
        }));
    }
    Ok(url.to_string())
}

fn validate_events(events: &[String]) -> ApiResult<String> {
    let known: Vec<&str> = EVENTS.iter().map(|(n, _)| *n).collect();
    let picked: Vec<&String> = events
        .iter()
        .filter(|e| known.contains(&e.as_str()))
        .collect();
    if picked.is_empty() {
        return Err(ApiError::bad("Pick at least one event."));
    }
    Ok(serde_json::to_string(&picked).unwrap_or_else(|_| "[]".into()))
}

async fn create_webhook(
    State(app): State<AppRef>,
    owner: Owner,
    Json(b): Json<WebhookIn>,
) -> ApiResult<Response> {
    let url = validate_url(&app, b.url.as_deref().unwrap_or(""))?;
    let events = validate_events(b.events.as_deref().unwrap_or(&[]))?;
    let description = b
        .description
        .map(|d| clip(d.trim(), 200))
        .filter(|d| !d.is_empty());
    let id = app.db.with(|c| {
        c.execute(
            "insert into webhook_endpoints (url, description, events, secret, created_at) values (?1, ?2, ?3, ?4, ?5)",
            (&url, &description, &events, format!("whsec_{}", token(30)), now()),
        )?;
        Ok(c.last_insert_rowid())
    })?;
    notify::audit(&app, "webhook.created", Some(&url), None, None, &owner.ip);
    Ok((StatusCode::CREATED, Json(endpoint_view(&app, id)?)).into_response())
}

async fn update_webhook(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
    Json(b): Json<WebhookIn>,
) -> ApiResult<Json<Value>> {
    endpoint_view(&app, id)?;
    if let Some(url) = b.url {
        let url = validate_url(&app, &url)?;
        app.db.with(|c| {
            c.execute(
                "update webhook_endpoints set url = ?1 where id = ?2",
                (url, id),
            )
        })?;
    }
    if let Some(d) = b.description {
        let d = Some(clip(d.trim(), 200)).filter(|d| !d.is_empty());
        app.db.with(|c| {
            c.execute(
                "update webhook_endpoints set description = ?1 where id = ?2",
                (d, id),
            )
        })?;
    }
    if let Some(events) = b.events {
        let events = validate_events(&events)?;
        app.db.with(|c| {
            c.execute(
                "update webhook_endpoints set events = ?1 where id = ?2",
                (events, id),
            )
        })?;
    }
    if let Some(enabled) = b.enabled {
        app.db.with(|c| {
            c.execute(
                "update webhook_endpoints set enabled = ?1, consecutive_failures = 0 where id = ?2",
                (enabled, id),
            )
        })?;
    }
    Ok(Json(endpoint_view(&app, id)?))
}

async fn delete_webhook(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let url = endpoint_view(&app, id)?["url"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    app.db
        .with(|c| c.execute("delete from webhook_endpoints where id = ?1", [id]))?;
    notify::audit(&app, "webhook.deleted", Some(&url), None, None, &owner.ip);
    Ok(no_content())
}

async fn webhook_secret(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let secret: Option<String> = app.db.with(|c| {
        c.query_row(
            "select secret from webhook_endpoints where id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()
    })?;
    Ok(Json(
        json!({ "secret": secret.ok_or_else(ApiError::not_found)? }),
    ))
}

async fn roll_secret(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let secret = format!("whsec_{}", token(30));
    let n = app.db.with(|c| {
        c.execute(
            "update webhook_endpoints set secret = ?1 where id = ?2",
            (&secret, id),
        )
    })?;
    if n == 0 {
        return Err(ApiError::not_found());
    }
    notify::audit(&app, "webhook.secret_rolled", None, None, None, &owner.ip);
    Ok(Json(json!({ "secret": secret })))
}

async fn test_webhook(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    endpoint_view(&app, id)?;
    notify::send(
        &app,
        id,
        TEST_EVENT,
        json!({ "machine": "example-machine", "project": "example-project", "agent": "claude", "reason": "test" }),
    );
    Ok(accepted(json!({})))
}

async fn deliveries(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    endpoint_view(&app, id)?;
    let list: Vec<Value> = app.db.with(|c| {
        let mut st = c.prepare(
            "select id, event, response_code, ok, attempt, duration_ms, created_at, delivered_at
             from webhook_deliveries where endpoint_id = ?1 order by id desc limit 50",
        )?;
        let rows = st.query_map([id], |r| {
            let delivered: Option<String> = r.get(7)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "event": r.get::<_, String>(1)?,
                "response_code": r.get::<_, Option<String>>(2)?,
                "ok": r.get::<_, i64>(3)? != 0,
                "pending": delivered.is_none(),
                "attempt": r.get::<_, i64>(4)?,
                "duration_ms": r.get::<_, Option<i64>>(5)?,
                "created_at": r.get::<_, String>(6)?,
                "delivered_at": delivered,
            }))
        })?;
        rows.collect()
    })?;
    Ok(Json(json!(list)))
}

async fn redeliver(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    notify::redeliver(&app, id).ok_or_else(ApiError::not_found)?;
    Ok(accepted(json!({})))
}

// ---- account ---------------------------------------------------------------------------

pub(crate) fn owner_row(app: &AppRef) -> ApiResult<(String, String)> {
    app.db
        .with(|c| {
            c.query_row(
                "select password_hash, totp_secret from owner where id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })?
        .ok_or_else(|| ApiError::conflict("This server has no owner yet."))
}

#[derive(Deserialize)]
struct PasswordChange {
    current_password: String,
    new_password: String,
    totp_code: String,
}

async fn change_password(
    State(app): State<AppRef>,
    owner: Owner,
    Json(b): Json<PasswordChange>,
) -> ApiResult<Response> {
    let (hash, secret) = owner_row(&app)?;
    if !auth::verify_password(&b.current_password, &hash)
        || !auth::check_totp(&secret, &b.totp_code)
    {
        return Err(ApiError::unauthorized(
            "The current password or the code isn't right.",
        ));
    }
    if b.new_password.chars().count() < MIN_PASSWORD {
        return Err(ApiError::bad(format!(
            "Use at least {MIN_PASSWORD} characters."
        )));
    }
    let new_hash = auth::hash_password(&b.new_password)
        .map_err(|_| ApiError::bad("Couldn't store the password."))?;
    let others: Vec<i64> = app.db.with(|c| {
        c.execute(
            "update owner set password_hash = ?1 where id = 1",
            [&new_hash],
        )?;
        let mut st = c.prepare("select id from web_sessions where id != ?1")?;
        let ids = st
            .query_map([owner.session_id], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        c.execute(
            "delete from web_sessions where id != ?1",
            [owner.session_id],
        )?;
        Ok(ids)
    })?;
    for id in others {
        app.hub.close_web_session(id);
    }
    notify::audit(
        &app,
        "account.password_changed",
        None,
        None,
        None,
        &owner.ip,
    );
    Ok(no_content())
}

async fn totp_start(State(app): State<AppRef>, owner: Owner) -> ApiResult<Json<Value>> {
    let secret = auth::new_totp_secret();
    app.pending_totp
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(owner.session_id, secret.clone());
    Ok(Json(auth::enrolment(&secret, &app.cfg.server_name)))
}

#[derive(Deserialize)]
struct TotpFinish {
    password: String,
    totp_code: String,
}

async fn totp_finish(
    State(app): State<AppRef>,
    owner: Owner,
    Json(b): Json<TotpFinish>,
) -> ApiResult<Response> {
    let (hash, _) = owner_row(&app)?;
    let pending = app
        .pending_totp
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&owner.session_id)
        .cloned();
    let Some(secret) = pending else {
        return Err(ApiError::conflict(
            "Start setting up the new authenticator first.",
        ));
    };
    if !auth::verify_password(&b.password, &hash) {
        return Err(ApiError::unauthorized("The password isn't right."));
    }
    if !auth::check_totp(&secret, &b.totp_code) {
        return Err(ApiError::unauthorized(
            "That code doesn't match the new authenticator.",
        ));
    }
    app.db
        .with(|c| c.execute("update owner set totp_secret = ?1 where id = 1", [&secret]))?;
    app.pending_totp
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&owner.session_id);
    notify::audit(&app, "account.totp_changed", None, None, None, &owner.ip);
    Ok(no_content())
}

async fn web_sessions(State(app): State<AppRef>, owner: Owner) -> ApiResult<Json<Value>> {
    let list: Vec<Value> = app.db.with(|c| {
        let mut st = c.prepare("select id, user_agent, ip, created_at, last_seen_at from web_sessions order by last_seen_at desc")?;
        let rows = st.query_map([], |r| {
            let id: i64 = r.get(0)?;
            Ok(json!({
                "id": id,
                "user_agent": r.get::<_, Option<String>>(1)?,
                "ip": r.get::<_, Option<String>>(2)?,
                "created_at": r.get::<_, String>(3)?,
                "last_seen_at": r.get::<_, String>(4)?,
                "current": id == owner.session_id,
            }))
        })?;
        rows.collect()
    })?;
    Ok(Json(json!(list)))
}

async fn end_web_session(
    State(app): State<AppRef>,
    owner: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let n = app
        .db
        .with(|c| c.execute("delete from web_sessions where id = ?1", [id]))?;
    if n == 0 {
        return Err(ApiError::not_found());
    }
    app.hub.close_web_session(id);
    notify::audit(
        &app,
        "account.signed_out_device",
        None,
        None,
        None,
        &owner.ip,
    );
    Ok(no_content())
}

#[derive(Deserialize)]
struct AuditQuery {
    before: Option<i64>,
}

async fn audit(
    State(app): State<AppRef>,
    _o: Owner,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Value>> {
    let before = q.before.unwrap_or(i64::MAX);
    let entries: Vec<Value> = app.db.with(|c| {
        let mut st = c.prepare(
            "select id, action, detail, machine, session_title, ip, created_at from audit_log where id < ?1 order by id desc limit 51",
        )?;
        let rows = st.query_map([before], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "action": r.get::<_, String>(1)?,
                "detail": r.get::<_, Option<String>>(2)?,
                "machine": r.get::<_, Option<String>>(3)?,
                "session_title": r.get::<_, Option<String>>(4)?,
                "ip": r.get::<_, Option<String>>(5)?,
                "created_at": r.get::<_, String>(6)?,
            }))
        })?;
        rows.collect()
    })?;
    let mut entries = entries;
    let next = if entries.len() > 50 {
        entries.truncate(50);
        entries.last().and_then(|e| e["id"].as_i64())
    } else {
        None
    };
    Ok(Json(json!({ "entries": entries, "next": next })))
}

// ---- push ------------------------------------------------------------------------------

async fn push_list(State(app): State<AppRef>, _o: Owner) -> ApiResult<Json<Value>> {
    let subs: Vec<Value> = app.db.with(|c| {
        let mut st = c.prepare(
            "select id, label, created_at, endpoint from push_subscriptions order by id desc",
        )?;
        let rows = st.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "label": r.get::<_, Option<String>>(1)?,
                "created_at": r.get::<_, String>(2)?,
                "endpoint": r.get::<_, String>(3)?,
            }))
        })?;
        rows.collect()
    })?;
    Ok(Json(
        json!({ "public_key": notify::vapid_public(&app.vapid), "subscriptions": subs }),
    ))
}

#[derive(Deserialize)]
struct PushKeys {
    p256dh: String,
    auth: String,
}

#[derive(Deserialize)]
struct PushIn {
    endpoint: String,
    keys: PushKeys,
    label: Option<String>,
}

async fn push_add(
    State(app): State<AppRef>,
    owner: Owner,
    Json(b): Json<PushIn>,
) -> ApiResult<Response> {
    let ok = url::Url::parse(&b.endpoint).is_ok_and(|u| u.scheme() == "https")
        && b.endpoint.len() < 2048;
    if !ok || b.keys.p256dh.len() > 200 || b.keys.auth.len() > 100 {
        return Err(ApiError::bad("That isn't a push subscription."));
    }
    let label = b.label.map(|l| clip(&l, 100));
    let id = app.db.with(|c| {
        c.execute(
            "insert into push_subscriptions (endpoint, p256dh, auth, label, created_at) values (?1, ?2, ?3, ?4, ?5)
             on conflict(endpoint) do update set p256dh = excluded.p256dh, auth = excluded.auth, label = excluded.label",
            (&b.endpoint, &b.keys.p256dh, &b.keys.auth, &label, now()),
        )?;
        c.query_row("select id from push_subscriptions where endpoint = ?1", [&b.endpoint], |r| r.get::<_, i64>(0))
    })?;
    notify::audit(
        &app,
        "push.device_added",
        label.as_deref(),
        None,
        None,
        &owner.ip,
    );
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))).into_response())
}

async fn push_remove(
    State(app): State<AppRef>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let n = app
        .db
        .with(|c| c.execute("delete from push_subscriptions where id = ?1", [id]))?;
    if n == 0 {
        return Err(ApiError::not_found());
    }
    Ok(no_content())
}

async fn push_test(State(app): State<AppRef>, _o: Owner) -> ApiResult<Response> {
    notify::push_all(
        &app,
        json!({ "title": "Termlab Server", "body": "Notifications work on this device.", "url": "/remote" }),
    );
    Ok(accepted(json!({})))
}
