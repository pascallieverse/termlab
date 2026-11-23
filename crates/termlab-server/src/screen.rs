//! Screen sessions: seeing and controlling a machine's screen from the browser.
//!
//! 1. The browser asks `POST /api/machines/{id}/screen` with a fresh authenticator code
//!    (unless it gave one in the last ten minutes). The machine must have screen sharing
//!    switched on in its Termlab, which the server can't change. The server picks a session
//!    id and tells the machine (`screen.open`).
//! 2. The machine starts `termlab-screen`, which opens its own websocket
//!    (`/api/machine/screen/ws?sid=`, machine token); the browser opens
//!    `/api/screen/ws?sid=` (cookie). Whichever comes first waits for the other.
//! 3. The server then relays: video and messages from the machine, input from the browser
//!    (only known message types, size-limited). It never decodes the video. Backpressure is
//!    end to end: the machine sends at most a few frames ahead of the browser's acks, and
//!    the relay only reads when it can write.
//!
//! Signing out, removing the machine, or the machine switching sharing off ends the session.
//! Each start and end is in the audit log.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};

use crate::auth::{Client, Owner};
use crate::util::{ApiError, ApiResult, clip, sha256_hex, token};
use crate::ws::MachineAuth;
use crate::{AppRef, notify};

/// A code from the authenticator app covers screen sessions started this long after it.
const VERIFIED_FOR: Duration = Duration::from_secs(10 * 60);
/// How long one side waits for the other to connect.
const RENDEZVOUS: Duration = Duration::from_secs(30);
const PING: Duration = Duration::from_secs(20);
/// A side that sends nothing for this long (not even a pong) is gone: end the session.
const SILENT: Duration = Duration::from_secs(60);
/// Largest message from the machine: an encoded key frame at the highest quality.
const MAX_FROM_MACHINE: usize = 8 << 20;
/// Largest message from the browser: input, or clipboard text sent to the machine.
const MAX_FROM_BROWSER: usize = (1 << 20) + 4096;
/// Message types a browser may send (the machine checks them again).
const BROWSER_TYPES: &[&str] = &[
    "ack",
    "keyframe",
    "display",
    "quality",
    "pointer",
    "wheel",
    "key",
    "text",
    "release",
    "clipboard.set",
    "clipboard.get",
];

/// What a machine said about screen sharing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Caps {
    /// Its Termlab can share the screen on this operating system.
    #[serde(default)]
    pub supported: bool,
    /// Switched on in its Termlab.
    #[serde(default)]
    pub enabled: bool,
}

struct Pending {
    machine: i64,
    web_session: i64,
    created: Instant,
    viewer: String,
    /// The machine's socket, when it came first.
    machine_socket: Option<WebSocket>,
    /// The browser waiting for the machine's socket, when it came first.
    waiter: Option<oneshot::Sender<Result<WebSocket, String>>>,
    /// The machine refused before the browser came.
    error: Option<String>,
}

struct Active {
    machine: i64,
    web_session: i64,
    stop: Arc<Notify>,
}

#[derive(Default)]
struct Inner {
    caps: HashMap<i64, Caps>,
    pending: HashMap<String, Pending>,
    active: HashMap<String, Active>,
    verified: HashMap<i64, Instant>,
}

#[derive(Default)]
pub struct ScreenHub {
    inner: Mutex<Inner>,
}

impl ScreenHub {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The machine reported its screen sharing state. Switching it off ends its sessions.
    pub fn set_caps(&self, machine: i64, caps: Caps) {
        let mut g = self.lock();
        if !caps.enabled {
            for a in g.active.values().filter(|a| a.machine == machine) {
                a.stop.notify_one();
            }
        }
        g.caps.insert(machine, caps);
    }

    /// For the machine list: `{ supported, enabled, active }`, or null for a Termlab
    /// without screen sharing.
    pub fn view(&self, machine: i64) -> Value {
        let g = self.lock();
        match g.caps.get(&machine) {
            Some(c) => json!({
                "supported": c.supported,
                "enabled": c.enabled,
                "active": g.active.values().filter(|a| a.machine == machine).count(),
            }),
            None => Value::Null,
        }
    }

    pub fn close_web_session(&self, web_session: i64) {
        let mut g = self.lock();
        g.verified.remove(&web_session);
        g.pending.retain(|_, p| p.web_session != web_session);
        for a in g.active.values().filter(|a| a.web_session == web_session) {
            a.stop.notify_one();
        }
    }

    pub fn close_machine(&self, machine: i64) {
        let mut g = self.lock();
        g.caps.remove(&machine);
        g.pending.retain(|_, p| p.machine != machine);
        for a in g.active.values().filter(|a| a.machine == machine) {
            a.stop.notify_one();
        }
    }

    /// The machine couldn't start the session.
    pub fn refused(&self, machine: i64, sid: &str, error: String) {
        let mut g = self.lock();
        let Some(p) = g.pending.get_mut(sid).filter(|p| p.machine == machine) else {
            return;
        };
        match p.waiter.take() {
            Some(w) => {
                let _ = w.send(Err(error));
                g.pending.remove(sid);
            }
            None => p.error = Some(error),
        }
    }

    fn sweep(g: &mut Inner) {
        g.pending
            .retain(|_, p| p.created.elapsed() < RENDEZVOUS * 2);
        g.verified.retain(|_, t| t.elapsed() < VERIFIED_FOR);
    }
}

// ---- routes ------------------------------------------------------------------------------

pub fn routes() -> Router<AppRef> {
    Router::new()
        .route("/api/machines/{id}/screen", post(start))
        .route("/api/screen/ws", get(browser_ws))
        .route("/api/machine/screen/ws", get(machine_ws))
}

#[derive(Deserialize)]
struct Start {
    totp_code: Option<String>,
}

fn machine_name(app: &AppRef, id: i64) -> ApiResult<Option<String>> {
    Ok(app.db.with(|c| {
        c.query_row("select name from machines where id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()
    })?)
}

/// A short "address · browser" for the prompt on the computer and the audit log.
fn viewer_label(client: &Client) -> String {
    let ua = &client.user_agent;
    let browser = ["Edg/", "Firefox/", "Chrome/", "Safari/"]
        .iter()
        .find(|b| ua.contains(**b))
        .map(|b| match *b {
            "Edg/" => "Edge",
            "Firefox/" => "Firefox",
            "Chrome/" => "Chrome",
            _ => "Safari",
        })
        .unwrap_or("a browser");
    let device = if ua.contains("iPhone") {
        " on iPhone"
    } else if ua.contains("iPad") {
        " on iPad"
    } else if ua.contains("Android") {
        " on Android"
    } else if ua.contains("Mac OS X") {
        " on a Mac"
    } else if ua.contains("Windows") {
        " on Windows"
    } else if ua.contains("Linux") {
        " on Linux"
    } else {
        ""
    };
    format!("{browser}{device} at {}", client.ip)
}

async fn start(
    State(app): State<AppRef>,
    o: Owner,
    client: Client,
    Path(id): Path<i64>,
    Json(b): Json<Start>,
) -> ApiResult<Response> {
    let Some(name) = machine_name(&app, id)? else {
        return Err(ApiError::not_found());
    };
    if !app.hub.machine_online(id) {
        return Err(ApiError::conflict("The computer is offline."));
    }
    let caps = app.hub.screen.lock().caps.get(&id).cloned();
    match caps {
        None => {
            return Err(ApiError::conflict(
                "This computer's Termlab can't share its screen. Update Termlab on it.",
            ));
        }
        Some(c) if !c.supported => {
            return Err(ApiError::conflict(
                "Screen sharing isn't available on this computer's operating system yet.",
            ));
        }
        Some(c) if !c.enabled => {
            return Err(ApiError::conflict(
                "Screen sharing is off on this computer. Switch it on there, in Termlab › Settings › Remote access.",
            ));
        }
        Some(_) => {}
    }

    // Step-up: a fresh authenticator code, unless one was given in the last few minutes.
    let fresh = app
        .hub
        .screen
        .lock()
        .verified
        .get(&o.session_id)
        .is_some_and(|t| t.elapsed() < VERIFIED_FOR);
    if !fresh {
        let Some(code) = b.totp_code.filter(|c| !c.trim().is_empty()) else {
            return Err(ApiError::new(
                StatusCode::PRECONDITION_REQUIRED,
                "Enter a code from your authenticator app to control this computer.",
            ));
        };
        if let Err(wait) = app.limiter.check(&client.ip) {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                format!(
                    "Too many attempts. Try again in {} minutes.",
                    wait.div_ceil(60)
                ),
            ));
        }
        let (_, secret) = crate::api::owner_row(&app)?;
        if !crate::auth::check_totp(&secret, &code) {
            app.limiter.failed(&client.ip);
            notify::audit(
                &app,
                "screen.denied",
                Some("wrong authenticator code"),
                Some(&name),
                None,
                &client.ip,
            );
            return Err(ApiError::bad("That code isn't right."));
        }
        app.limiter.succeeded(&client.ip);
        app.hub
            .screen
            .lock()
            .verified
            .insert(o.session_id, Instant::now());
    }

    let sid = sha256_hex(&token(32))[..32].to_string();
    let viewer = viewer_label(&client);
    {
        let mut g = app.hub.screen.lock();
        ScreenHub::sweep(&mut g);
        g.pending.insert(
            sid.clone(),
            Pending {
                machine: id,
                web_session: o.session_id,
                created: Instant::now(),
                viewer: viewer.clone(),
                machine_socket: None,
                waiter: None,
                error: None,
            },
        );
    }
    if !app.hub.to_machine(
        id,
        &json!({ "type": "screen.open", "sid": sid, "viewer": viewer }),
    ) {
        app.hub.screen.lock().pending.remove(&sid);
        return Err(ApiError::conflict("The computer is offline."));
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "sid": sid }))).into_response())
}

#[derive(Deserialize)]
struct SidQuery {
    sid: String,
}

async fn machine_ws(
    State(app): State<AppRef>,
    m: MachineAuth,
    Query(q): Query<SidQuery>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    let known = app
        .hub
        .screen
        .lock()
        .pending
        .get(&q.sid)
        .is_some_and(|p| p.machine == m.id && p.machine_socket.is_none());
    if !known {
        return Err(ApiError::not_found());
    }
    let sid = q.sid;
    Ok(ws
        .max_message_size(MAX_FROM_MACHINE + 1024)
        .max_frame_size(MAX_FROM_MACHINE + 1024)
        .on_upgrade(move |socket| async move {
            let mut g = app.hub.screen.lock();
            let Some(p) = g.pending.get_mut(&sid) else {
                return; // Expired or cancelled: dropping the socket closes it.
            };
            match p.waiter.take() {
                Some(w) => {
                    g.pending.remove(&sid);
                    let _ = w.send(Ok(socket));
                }
                None => p.machine_socket = Some(socket),
            }
        }))
}

async fn browser_ws(
    State(app): State<AppRef>,
    owner: Owner,
    client: Client,
    headers: HeaderMap,
    Query(q): Query<SidQuery>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    if !crate::ws::origin_allowed(&app, &headers) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Cross-origin websocket refused.",
        ));
    }
    let (rx, ready, machine, viewer) = {
        let mut g = app.hub.screen.lock();
        let Some(p) = g
            .pending
            .get_mut(&q.sid)
            .filter(|p| p.web_session == owner.session_id && p.waiter.is_none())
        else {
            return Err(ApiError::not_found());
        };
        let machine = p.machine;
        let viewer = p.viewer.clone();
        if let Some(e) = p.error.take() {
            g.pending.remove(&q.sid);
            (None, Some(Err(e)), machine, viewer)
        } else if let Some(s) = p.machine_socket.take() {
            g.pending.remove(&q.sid);
            (None, Some(Ok(s)), machine, viewer)
        } else {
            let (tx, rx) = oneshot::channel();
            p.waiter = Some(tx);
            (Some(rx), None, machine, viewer)
        }
    };
    let sid = q.sid;
    let web_session = owner.session_id;
    Ok(ws
        .max_message_size(MAX_FROM_BROWSER)
        .on_upgrade(move |mut browser| async move {
            let machine_socket = match ready {
                Some(r) => r,
                None => {
                    let rx = rx.expect("waiting");
                    match tokio::time::timeout(RENDEZVOUS, rx).await {
                        Ok(Ok(r)) => r,
                        _ => Err("The computer didn't answer. Is its Termlab up to date?".into()),
                    }
                }
            };
            app.hub.screen.lock().pending.remove(&sid);
            let machine_socket = match machine_socket {
                Ok(s) => s,
                Err(e) => {
                    let _ = browser
                        .send(Message::Text(
                            json!({ "type": "error", "message": e }).to_string().into(),
                        ))
                        .await;
                    let _ = browser.send(Message::Close(None)).await;
                    return;
                }
            };
            relay(
                app,
                sid,
                machine,
                web_session,
                viewer,
                client.ip,
                browser,
                machine_socket,
            )
            .await;
        }))
}

#[allow(clippy::too_many_arguments)]
async fn relay(
    app: AppRef,
    sid: String,
    machine: i64,
    web_session: i64,
    viewer: String,
    ip: String,
    browser: WebSocket,
    machine_socket: WebSocket,
) {
    let stop = Arc::new(Notify::new());
    app.hub.screen.lock().active.insert(
        sid.clone(),
        Active {
            machine,
            web_session,
            stop: stop.clone(),
        },
    );
    let name = machine_name(&app, machine).ok().flatten();
    notify::audit(
        &app,
        "screen.started",
        Some(&viewer),
        name.as_deref(),
        None,
        &ip,
    );
    broadcast_machine(&app, machine);
    let started = Instant::now();

    let (mut to_browser, mut from_browser) = browser.split();
    let (mut to_machine, mut from_machine) = machine_socket.split();

    let down = async {
        let mut ping = tokio::time::interval(PING);
        ping.reset();
        let mut heard = Instant::now();
        loop {
            tokio::select! {
                msg = from_machine.next() => {
                    heard = Instant::now();
                    match msg {
                        Some(Ok(Message::Binary(b))) if b.len() <= MAX_FROM_MACHINE => {
                            if to_browser.send(Message::Binary(b)).await.is_err() { break }
                        }
                        Some(Ok(Message::Text(t))) if t.len() <= MAX_FROM_MACHINE => {
                            if to_browser.send(Message::Text(t)).await.is_err() { break }
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    }
                }
                _ = ping.tick() => {
                    if heard.elapsed() > SILENT || to_browser.send(Message::Ping(Vec::new().into())).await.is_err() { break }
                }
            }
        }
        let _ = to_browser.send(Message::Close(None)).await;
    };
    let up = async {
        let mut ping = tokio::time::interval(PING);
        ping.reset();
        let mut heard = Instant::now();
        loop {
            tokio::select! {
                msg = from_browser.next() => {
                    heard = Instant::now();
                    match msg {
                        Some(Ok(Message::Text(t))) => {
                            let ok = serde_json::from_str::<Value>(t.as_str()).ok().is_some_and(|v| {
                                v["type"].as_str().is_some_and(|k| BROWSER_TYPES.contains(&k))
                            });
                            if ok && to_machine.send(Message::Text(t)).await.is_err() { break }
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    }
                }
                _ = ping.tick() => {
                    if heard.elapsed() > SILENT || to_machine.send(Message::Ping(Vec::new().into())).await.is_err() { break }
                }
            }
        }
        let _ = to_machine.send(Message::Close(None)).await;
    };
    tokio::select! {
        _ = down => {}
        _ = up => {}
        _ = stop.notified() => {}
    }

    app.hub.screen.lock().active.remove(&sid);
    let mins = started.elapsed().as_secs().div_ceil(60);
    notify::audit(
        &app,
        "screen.ended",
        Some(&clip(&format!("{viewer}, {mins} min"), 200)),
        name.as_deref(),
        None,
        &ip,
    );
    broadcast_machine(&app, machine);
}

fn broadcast_machine(app: &AppRef, machine: i64) {
    if let Ok(Some(m)) = crate::sessions::machine_view(app, machine) {
        app.hub
            .broadcast(&json!({ "type": "machine.updated", "machine": m }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(ua: &str) -> Client {
        Client {
            ip: "203.0.113.5".into(),
            user_agent: ua.into(),
        }
    }

    #[test]
    fn viewer_labels_name_browser_and_device() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
        assert_eq!(
            viewer_label(&client(iphone)),
            "Safari on iPhone at 203.0.113.5"
        );
        let chrome = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
        assert_eq!(
            viewer_label(&client(chrome)),
            "Chrome on Windows at 203.0.113.5"
        );
    }

    #[test]
    fn switching_sharing_off_stops_sessions() {
        let hub = ScreenHub::default();
        let stop = Arc::new(Notify::new());
        hub.lock().active.insert(
            "s".into(),
            Active {
                machine: 3,
                web_session: 1,
                stop: stop.clone(),
            },
        );
        hub.set_caps(
            3,
            Caps {
                supported: true,
                enabled: false,
            },
        );
        // The stop was signalled: a waiter returns at once.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            tokio::time::timeout(Duration::from_millis(100), stop.notified())
                .await
                .expect("stopped");
        });
        assert_eq!(hub.view(3)["active"], 1);
    }

    #[test]
    fn a_refusal_reaches_a_waiting_browser() {
        let hub = ScreenHub::default();
        let (tx, mut rx) = oneshot::channel();
        hub.lock().pending.insert(
            "s".into(),
            Pending {
                machine: 3,
                web_session: 1,
                created: Instant::now(),
                viewer: String::new(),
                machine_socket: None,
                waiter: Some(tx),
                error: None,
            },
        );
        // Another machine can't answer for it.
        hub.refused(4, "s", "no".into());
        assert!(rx.try_recv().is_err());
        hub.refused(3, "s", "off".into());
        assert_eq!(rx.try_recv().unwrap().err().as_deref(), Some("off"));
        assert!(hub.lock().pending.is_empty());
    }
}
