//! Websockets: paired machines (`/api/machine/ws`, bearer token) and signed-in browsers
//! (`/api/ws`, session cookie), plus pairing (`/api/pair`).

use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, State};
use axum::http::header::{AUTHORIZATION, ORIGIN};
use axum::http::{HeaderMap, StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::unbounded_channel;

use crate::auth::{Client, Owner};
use crate::hub::Outgoing;
use crate::util::{ApiError, ApiResult, clip, now, sha256_hex, token, unix_now};
use crate::{AppRef, api, notify, sessions};

const PING: Duration = Duration::from_secs(25);
/// A machine that stays disconnected this long counts as offline (webhook).
const OFFLINE_AFTER: Duration = Duration::from_secs(90);
const MAX_MESSAGE: usize = 1 << 20;
/// Terminal bytes per output message (base64), generously above what termlabd sends.
const MAX_OUTPUT_B64: usize = 256 * 1024;

pub fn routes() -> Router<AppRef> {
    Router::new()
        .route("/api/pair", post(pair))
        .route("/api/machine/ws", get(machine_ws))
        .route("/api/machine", axum::routing::delete(unpair_self))
        .route("/api/ws", get(browser_ws))
}

// ---- pairing ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Pair {
    code: String,
    name: String,
    os: Option<String>,
    version: Option<String>,
}

async fn pair(
    State(app): State<AppRef>,
    client: Client,
    Json(b): Json<Pair>,
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
    let code = api::normalize_code(&b.code);
    let name = clip(b.name.trim(), 100);
    if name.is_empty() {
        return Err(ApiError::bad("Give this computer a name."));
    }
    let raw = token(32);
    let t = now();
    let machine_id: Option<i64> = app.db.with(|c| {
        let found: Option<i64> = c
            .query_row(
                "select expires_at from pairing_codes where code_hash = ?1",
                [sha256_hex(&code)],
                |r| r.get(0),
            )
            .optional()?;
        if found.is_none_or(|exp| exp <= unix_now()) {
            return Ok(None);
        }
        c.execute("delete from pairing_codes", [])?;
        c.execute(
            "insert into machines (name, os, version, token_hash, online, last_seen_at, created_at) values (?1, ?2, ?3, ?4, 0, ?5, ?5)",
            (&name, b.os.map(|o| clip(&o, 100)), b.version.map(|v| clip(&v, 40)), sha256_hex(&raw), &t),
        )?;
        Ok(Some(c.last_insert_rowid()))
    })?;
    let Some(machine_id) = machine_id else {
        app.limiter.failed(&client.ip);
        return Err(ApiError::bad(
            "This pairing code is not valid or has expired.",
        ));
    };
    notify::audit(&app, "machine.paired", None, Some(&name), None, &client.ip);
    if let Some(m) = sessions::machine_view(&app, machine_id)? {
        app.hub
            .broadcast(&json!({ "type": "machine.updated", "machine": m }));
    }
    Ok((
        StatusCode::CREATED,
        Json(json!({ "machine_id": machine_id, "token": raw, "server_name": app.cfg.server_name })),
    )
        .into_response())
}

// ---- machines --------------------------------------------------------------------------

/// A paired machine, from its bearer token.
pub struct MachineAuth {
    pub id: i64,
}

impl FromRequestParts<AppRef> for MachineAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &AppRef) -> Result<Self, Self::Rejection> {
        let raw = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| ApiError::unauthorized("Missing machine token."))?;
        let id: Option<i64> = app.db.with(|c| {
            c.query_row(
                "select id from machines where token_hash = ?1",
                [sha256_hex(raw.trim())],
                |r| r.get(0),
            )
            .optional()
        })?;
        id.map(|id| MachineAuth { id }).ok_or_else(|| {
            ApiError::unauthorized("This computer isn't paired with this server any more.")
        })
    }
}

/// Termlab unpairs itself: its token is revoked and the machine removed.
async fn unpair_self(
    State(app): State<AppRef>,
    m: MachineAuth,
    client: Client,
) -> ApiResult<Response> {
    let name: Option<String> = app.db.with(|c| {
        c.query_row("select name from machines where id = ?1", [m.id], |r| {
            r.get(0)
        })
        .optional()
    })?;
    let sessions: Vec<String> = app.db.with(|c| {
        let mut st = c.prepare("select id from sessions where machine_id = ?1")?;
        st.query_map([m.id], |r| r.get(0))?.collect()
    })?;
    app.db
        .with(|c| c.execute("delete from machines where id = ?1", [m.id]))?;
    app.hub.revoke_machine(m.id);
    for id in sessions {
        app.hub
            .broadcast(&json!({ "type": "session.removed", "id": id }));
    }
    notify::audit(
        &app,
        "machine.unpaired",
        None,
        name.as_deref(),
        None,
        &client.ip,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn machine_ws(State(app): State<AppRef>, m: MachineAuth, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| machine_socket(app, m.id, socket))
}

fn set_online(app: &AppRef, machine: i64, online: bool) {
    let _ = app.db.with(|c| {
        c.execute(
            "update machines set online = ?1, last_seen_at = ?2 where id = ?3",
            (online, now(), machine),
        )
    });
    if let Ok(Some(m)) = sessions::machine_view(app, machine) {
        app.hub
            .broadcast(&json!({ "type": "machine.updated", "machine": m }));
    }
}

async fn machine_socket(app: AppRef, machine: i64, socket: WebSocket) {
    let conn = app.hub.next_id();
    let (tx, mut rx) = unbounded_channel();
    app.hub.machine_connected(machine, conn, tx);
    set_online(&app, machine, true);
    tracing::info!(machine, "machine connected");

    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING);
    ping.reset();
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(Outgoing::Text(t)) => { if sink.send(Message::Text(t.into())).await.is_err() { break } }
                Some(Outgoing::Close) | None => { let _ = sink.send(Message::Close(None)).await; break }
            },
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    handle_machine(&app, machine, t.as_str());
                    if last_seen.elapsed() > Duration::from_secs(30) {
                        last_seen = Instant::now();
                        let _ = app.db.with(|c| c.execute("update machines set last_seen_at = ?1 where id = ?2", (now(), machine)));
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => { if sink.send(Message::Ping(Vec::new().into())).await.is_err() { break } }
        }
    }

    if let Some(generation) = app.hub.machine_disconnected(machine, conn) {
        set_online(&app, machine, false);
        tracing::info!(machine, "machine disconnected");
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(OFFLINE_AFTER).await;
            if app.hub.machine_generation(machine) != generation {
                return; // It came back.
            }
            let row: Option<(String, Option<String>)> = app
                .db
                .with(|c| {
                    c.query_row(
                        "select name, last_seen_at from machines where id = ?1",
                        [machine],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                })
                .ok()
                .flatten();
            if let Some((name, last_seen)) = row {
                notify::machine_offline(&app, &name, last_seen);
            }
        });
    }
}

fn handle_machine(app: &AppRef, machine: i64, text: &str) {
    let Ok(msg) = serde_json::from_str::<Value>(text) else {
        return;
    };
    match msg["type"].as_str().unwrap_or("") {
        "hello" => {
            let os = msg["os"].as_str().map(|s| clip(s, 100));
            let version = msg["version"].as_str().map(|s| clip(s, 40));
            let _ = app.db.with(|c| {
                c.execute(
                    "update machines set os = coalesce(?1, os), version = coalesce(?2, version) where id = ?3",
                    (os, version, machine),
                )
            });
            if let Ok(Some(m)) = sessions::machine_view(app, machine) {
                app.hub
                    .broadcast(&json!({ "type": "machine.updated", "machine": m }));
            }
        }
        "snapshot" => {
            let list = msg["sessions"].as_array().cloned().unwrap_or_default();
            sessions::snapshot(app, machine, list);
            refresh_machine(app, machine);
        }
        "session" => {
            sessions::upsert(app, machine, msg["session"].clone(), false);
            refresh_machine(app, machine);
        }
        "removed" => {
            if let Some(id) = msg["id"].as_str() {
                sessions::remove(app, machine, id);
                refresh_machine(app, machine);
            }
        }
        "output" => {
            let (Some(id), Some(data)) = (msg["id"].as_str(), msg["data"].as_str()) else {
                return;
            };
            if data.len() > MAX_OUTPUT_B64 {
                return;
            }
            // Pass on only the known fields.
            app.hub.output(
                machine,
                &json!({
                    "type": "output", "id": id, "data": data,
                    "snapshot": msg["snapshot"].as_bool().unwrap_or(false),
                    "cols": msg["cols"].as_u64().filter(|c| *c > 0 && *c < 1000),
                    "rows": msg["rows"].as_u64().filter(|r| *r > 0 && *r < 1000),
                }),
            );
        }
        _ => {}
    }
}

/// Session counts on the machine card follow session changes.
fn refresh_machine(app: &AppRef, machine: i64) {
    if let Ok(Some(m)) = sessions::machine_view(app, machine) {
        app.hub
            .broadcast(&json!({ "type": "machine.updated", "machine": m }));
    }
}

// ---- browsers --------------------------------------------------------------------------

/// Same-origin only: a page on another site can't open this websocket with our cookie.
fn origin_allowed(app: &AppRef, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true; // Not a browser.
    };
    if origin == app.cfg.origin() {
        return true;
    }
    // Local development (a dev server proxying to a server on this machine).
    let host = |s: &str| {
        url::Url::parse(s)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
    };
    let local =
        |h: Option<String>| h.is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h == "[::1]");
    local(host(origin)) && local(host(&app.cfg.public_url))
}

async fn browser_ws(
    State(app): State<AppRef>,
    owner: Owner,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&app, &headers) {
        return ApiError::new(StatusCode::FORBIDDEN, "Cross-origin websocket refused.")
            .into_response();
    }
    ws.max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| browser_socket(app, owner.session_id, socket))
}

#[derive(Deserialize)]
struct BrowserMsg {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    on: Option<bool>,
    data: Option<String>,
}

async fn browser_socket(app: AppRef, web_session: i64, socket: WebSocket) {
    let conn = app.hub.next_id();
    let (tx, mut rx) = unbounded_channel();
    app.hub.browser_connected(conn, web_session, tx);
    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING);
    ping.reset();
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(Outgoing::Text(t)) => { if sink.send(Message::Text(t.into())).await.is_err() { break } }
                Some(Outgoing::Close) | None => { let _ = sink.send(Message::Close(None)).await; break }
            },
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    let Ok(m) = serde_json::from_str::<BrowserMsg>(t.as_str()) else { continue };
                    let Some(id) = m.id else { continue };
                    match m.kind.as_str() {
                        "watch" => {
                            if let Ok(Some((_, machine, _))) = sessions::find(&app, &id) {
                                app.hub.watch(conn, &id, machine, m.on.unwrap_or(false));
                            }
                        }
                        "input" => {
                            if let Some(data) = m.data.filter(|d| d.len() <= 64 * 1024) {
                                app.hub.input(conn, &id, &data);
                            }
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => { if sink.send(Message::Ping(Vec::new().into())).await.is_err() { break } }
        }
    }
    app.hub.browser_disconnected(conn);
}
