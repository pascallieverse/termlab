//! The remote-access bridge: links this computer's sessions to an ExampleLab account.
//!
//! - Session state goes up over HTTPS (the device API); the server diffs it and fires
//!   webhooks and phone notifications.
//! - Commands (answer, message, stop, hold) come down on `private-machine.{id}`.
//! - Terminal bytes go to browsers as client events on `presence-terminal.{session}`,
//!   only while someone is watching. They pass through the websocket server, never PHP.
//!
//! The websocket server (Laravel Reverb) speaks the Pusher protocol. The protocol is
//! documented in the ExampleLab repository: `docs/termlab-remote-protocol.md`.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use termlab_core::ipc::replay_prefix;
use termlab_core::model::{
    ApprovalChoice, Folder, FolderId, PromptKind, SessionId, SessionOrigin, SessionView, now_ms,
};
use termlab_core::pty::PtyEvent;
use termlab_core::remote::{PairRequest, RemoteControl, RemoteFuture, RemoteState, RemoteStatus};
use termlab_core::{CoreEvent, DataDirs, Termlab, git};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

const CONFIG_FILE: &str = "remote.json";
const HEARTBEAT: Duration = Duration::from_secs(30);
/// Pusher's activity timeout is 30 s on Reverb; ping a little more often.
const PING: Duration = Duration::from_secs(25);
/// A session that changes often is reported at most this often.
const DEBOUNCE: Duration = Duration::from_millis(300);
const FLUSH_TICK: Duration = Duration::from_millis(150);
/// Sessions shown remotely: live ones, plus recent ones that finished.
const RECENT_MS: i64 = 48 * 3600 * 1000;
const MAX_SESSIONS: usize = 200;
const PREVIEW_LINES: usize = 12;
/// Raw terminal bytes per websocket message. Base64 plus JSON stays under Reverb's
/// default 10 KB message limit.
const CHUNK: usize = 6 * 1024;
const OUTPUT_BATCH: Duration = Duration::from_millis(40);
/// How much scrollback a viewer gets when they open a terminal.
const SNAPSHOT_MAX: usize = 512 * 1024;
const BRANCH_TTL: Duration = Duration::from_secs(60);
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Reverb {
    key: String,
    host: String,
    port: u16,
    scheme: String,
    auth_endpoint: String,
}

/// What pairing returned; kept in `<data-dir>/remote.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Paired {
    server_url: String,
    token: String,
    machine_id: u64,
    machine_name: String,
    account_name: Option<String>,
    plan: Option<String>,
    reverb: Reverb,
}

enum Fail {
    /// The token was revoked: the machine was removed in the dashboard.
    Revoked,
    /// The account has no plan with remote access.
    NoPlan,
    Other(String),
}

impl From<reqwest::Error> for Fail {
    fn from(e: reqwest::Error) -> Self {
        Fail::Other(network_error(&e))
    }
}

struct Inner {
    paired: Option<Paired>,
    state: RemoteState,
    error: Option<String>,
    sessions: usize,
    viewers: usize,
    task: Option<JoinHandle<()>>,
}

pub struct Bridge {
    tl: Arc<Termlab>,
    path: PathBuf,
    http: reqwest::Client,
    default_name: String,
    os: String,
    inner: Mutex<Inner>,
    branches: Mutex<HashMap<FolderId, (Option<String>, Instant)>>,
}

impl Bridge {
    pub fn new(tl: Arc<Termlab>, dirs: &DataDirs) -> anyhow::Result<Arc<Self>> {
        let path = dirs.root.join(CONFIG_FILE);
        let paired = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Paired>(&b).ok());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(format!("Termlab/{VERSION}"))
            // A redirect would turn POSTs into GETs; say where the server points instead.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let state = if paired.is_some() {
            RemoteState::Connecting
        } else {
            RemoteState::Unpaired
        };
        Ok(Arc::new(Self {
            tl,
            path,
            http,
            default_name: default_machine_name(),
            os: os_description(),
            inner: Mutex::new(Inner {
                paired,
                state,
                error: None,
                sessions: 0,
                viewers: 0,
                task: None,
            }),
            branches: Mutex::new(HashMap::new()),
        }))
    }

    /// Connects when this computer is paired.
    pub fn start(self: &Arc<Self>) {
        let mut inner = self.inner.lock().expect("remote lock");
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        if let Some(cfg) = inner.paired.clone() {
            let this = self.clone();
            inner.task = Some(tokio::spawn(async move { this.run(cfg).await }));
        }
    }

    fn set_state(&self, state: RemoteState, error: Option<String>) {
        let mut inner = self.inner.lock().expect("remote lock");
        inner.state = state;
        inner.error = error;
        if state != RemoteState::Connected {
            inner.viewers = 0;
        }
    }

    fn set_counts(&self, sessions: usize, viewers: usize) {
        let mut inner = self.inner.lock().expect("remote lock");
        inner.sessions = sessions;
        inner.viewers = viewers;
    }

    fn forget(&self) {
        let _ = std::fs::remove_file(&self.path);
        let mut inner = self.inner.lock().expect("remote lock");
        inner.paired = None;
        inner.sessions = 0;
        inner.viewers = 0;
    }

    /// Reconnects forever, backing off, until unpaired.
    async fn run(self: Arc<Self>, cfg: Paired) {
        let mut backoff = Duration::from_secs(2);
        loop {
            self.set_state(RemoteState::Connecting, None);
            let started = Instant::now();
            match self.connect_once(&cfg).await {
                Err(Fail::Revoked) => {
                    tracing::info!("remote: token revoked; unpairing");
                    self.forget();
                    self.set_state(
                        RemoteState::Unpaired,
                        Some("This computer was removed from your ExampleLab account.".into()),
                    );
                    return;
                }
                Err(Fail::NoPlan) => {
                    self.set_state(
                        RemoteState::NoPlan,
                        Some("Your ExampleLab account has no plan with remote access.".into()),
                    );
                    tokio::time::sleep(Duration::from_secs(300)).await;
                    continue;
                }
                Err(Fail::Other(msg)) => {
                    tracing::warn!(error = %msg, "remote: connection failed");
                    self.set_state(RemoteState::Error, Some(msg));
                }
                Ok(()) => {
                    tracing::info!("remote: disconnected; reconnecting");
                    self.set_state(RemoteState::Connecting, None);
                }
            }
            if started.elapsed() > Duration::from_secs(60) {
                backoff = Duration::from_secs(2);
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    }

    // ---- HTTP ------------------------------------------------------------------------

    async fn api(
        &self,
        cfg: &Paired,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, Fail> {
        let mut req = self
            .http
            .request(method, format!("{}/api/v1{path}", cfg.server_url))
            .bearer_auth(&cfg.token)
            .header("Accept", "application/json");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        match status.as_u16() {
            200..=299 => Ok(serde_json::from_str(&text).unwrap_or(Value::Null)),
            401 => Err(Fail::Revoked),
            402 => Err(Fail::NoPlan),
            _ => Err(Fail::Other(server_message(status, &text))),
        }
    }

    async fn heartbeat(&self, cfg: &Paired) -> Result<(), Fail> {
        self.api(
            cfg,
            reqwest::Method::POST,
            "/machine/heartbeat",
            Some(json!({ "version": VERSION, "os": self.os })),
        )
        .await
        .map(|_| ())
    }

    async fn channel_auth(
        &self,
        cfg: &Paired,
        socket_id: &str,
        channel: &str,
    ) -> Result<Value, Fail> {
        let res = self
            .http
            .post(&cfg.reverb.auth_endpoint)
            .bearer_auth(&cfg.token)
            .header("Accept", "application/json")
            .json(&json!({ "socket_id": socket_id, "channel_name": channel }))
            .send()
            .await?;
        match res.status().as_u16() {
            200..=299 => Ok(res.json().await?),
            401 => Err(Fail::Revoked),
            402 => Err(Fail::NoPlan),
            s => Err(Fail::Other(format!(
                "websocket auth for {channel} failed ({s})"
            ))),
        }
    }

    // ---- one connection ----------------------------------------------------------------

    async fn connect_once(self: &Arc<Self>, cfg: &Paired) -> Result<(), Fail> {
        let me = self
            .api(cfg, reqwest::Method::GET, "/machine", None)
            .await?;
        if me.get("remote").and_then(Value::as_bool) == Some(false) {
            return Err(Fail::NoPlan);
        }
        self.heartbeat(cfg).await?;

        let ws_url = format!(
            "{}://{}:{}/app/{}?protocol=7&client=termlab&version={VERSION}&flash=false",
            if cfg.reverb.scheme == "https" {
                "wss"
            } else {
                "ws"
            },
            cfg.reverb.host,
            cfg.reverb.port,
            cfg.reverb.key
        );
        let connector = if cfg.reverb.scheme == "https" {
            use rustls_platform_verifier::ConfigVerifierExt;
            let tls = rustls::ClientConfig::with_platform_verifier()
                .map_err(|e| Fail::Other(format!("TLS setup failed: {e}")))?;
            Some(tokio_tungstenite::Connector::Rustls(Arc::new(tls)))
        } else {
            None
        };
        let (ws, _) =
            tokio_tungstenite::connect_async_tls_with_config(&ws_url, None, false, connector)
                .await
                .map_err(|e| Fail::Other(format!("Couldn't open the live connection: {e}")))?;
        let (mut sink, mut stream) = ws.split();

        // The server says hello with our socket id.
        let socket_id = loop {
            let msg = tokio::time::timeout(Duration::from_secs(15), stream.next())
                .await
                .map_err(|_| Fail::Other("The live connection didn't answer.".into()))?;
            let Some(Ok(Message::Text(text))) = msg else {
                return Err(Fail::Other(
                    "The live connection closed while connecting.".into(),
                ));
            };
            let (event, _, data) = parse_frame(text.as_str());
            if event == "pusher:connection_established" {
                break data
                    .get("socket_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
            if event == "pusher:error" {
                return Err(Fail::Other(format!("Live connection refused: {data}")));
            }
        };

        let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
        let commands = format!("private-machine.{}", cfg.machine_id);
        let auth = self.channel_auth(cfg, &socket_id, &commands).await?;
        send(&mut sink, subscribe_frame(&commands, &auth)).await?;

        let mut core_rx = self.tl.subscribe();
        let mut synced: HashSet<SessionId> = HashSet::new();
        let mut viewers: HashMap<SessionId, HashSet<String>> = HashMap::new();
        let mut forwarders: HashMap<SessionId, JoinHandle<()>> = HashMap::new();
        let mut dirty: HashMap<SessionId, Instant> = HashMap::new();

        self.full_sync(cfg, &socket_id, &mut sink, &mut synced)
            .await?;
        self.set_state(RemoteState::Connected, None);
        self.set_counts(synced.len(), 0);
        tracing::info!(sessions = synced.len(), "remote: connected");

        let mut flush = tokio::time::interval(FLUSH_TICK);
        let mut heartbeat = tokio::time::interval(HEARTBEAT);
        heartbeat.reset();
        let mut ping = tokio::time::interval(PING);
        ping.reset();

        let result = loop {
            tokio::select! {
                msg = stream.next() => {
                    let text = match msg {
                        Some(Ok(Message::Text(t))) => t.to_string(),
                        Some(Ok(Message::Ping(p))) => { let _ = sink.send(Message::Pong(p)).await; continue }
                        Some(Ok(Message::Close(_))) | None => break Ok(()),
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => break Err(Fail::Other(format!("Live connection lost: {e}"))),
                    };
                    let (event, channel, data) = parse_frame(&text);
                    match event.as_str() {
                        "pusher:ping" => send(&mut sink, json!({"event": "pusher:pong", "data": {}}).to_string()).await?,
                        "pusher:error" => tracing::warn!(%data, "remote: server error"),
                        "pusher_internal:subscription_succeeded" | "pusher_internal:member_added" | "pusher_internal:member_removed" => {
                            if let Some(sid) = terminal_session(&channel) {
                                let set = viewers.entry(sid).or_default();
                                match event.as_str() {
                                    "pusher_internal:subscription_succeeded" => {
                                        set.clear();
                                        if let Some(ids) = data.pointer("/presence/ids").and_then(Value::as_array) {
                                            set.extend(ids.iter().filter_map(member_id).filter(|id| !id.starts_with("machine-")));
                                        }
                                    }
                                    "pusher_internal:member_added" => {
                                        if let Some(id) = data.get("user_id").and_then(member_id).filter(|id| !id.starts_with("machine-")) {
                                            set.insert(id);
                                        }
                                    }
                                    _ => {
                                        if let Some(id) = data.get("user_id").and_then(member_id) {
                                            set.remove(&id);
                                        }
                                    }
                                }
                                self.update_forwarder(sid, &viewers, &mut forwarders, &out_tx);
                                self.set_counts(synced.len(), viewers.values().map(HashSet::len).sum());
                            }
                        }
                        "command" if channel == commands => self.run_command(&data),
                        "client-input" => {
                            if let (Some(sid), Some(bytes)) = (terminal_session(&channel), data.get("data").and_then(Value::as_str).and_then(|d| B64.decode(d).ok())) {
                                let _ = self.tl.write(sid, bytes);
                            }
                        }
                        _ => {}
                    }
                }
                ev = core_rx.recv() => match ev {
                    Ok(CoreEvent::SessionUpdated { session }) => { dirty.entry(session.record.id).or_insert_with(Instant::now); }
                    Ok(CoreEvent::SessionRemoved { id }) => {
                        dirty.remove(&id);
                        if synced.remove(&id) {
                            self.drop_session(cfg, id, &mut sink, &mut viewers, &mut forwarders).await?;
                            self.set_counts(synced.len(), viewers.values().map(HashSet::len).sum());
                        }
                    }
                    Ok(CoreEvent::FoldersChanged) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        dirty.clear();
                        self.branches.lock().expect("branch lock").clear();
                        self.full_sync(cfg, &socket_id, &mut sink, &mut synced).await?;
                        for gone in viewers.keys().copied().filter(|id| !synced.contains(id)).collect::<Vec<_>>() {
                            self.drop_session(cfg, gone, &mut sink, &mut viewers, &mut forwarders).await?;
                        }
                        self.set_counts(synced.len(), viewers.values().map(HashSet::len).sum());
                    }
                    Err(broadcast::error::RecvError::Closed) => break Ok(()),
                },
                _ = flush.tick() => {
                    let due: Vec<SessionId> = dirty.iter().filter(|(_, t)| t.elapsed() >= DEBOUNCE).map(|(id, _)| *id).collect();
                    for id in due {
                        dirty.remove(&id);
                        match self.session_payload(id).await {
                            Some(view) => {
                                self.api(cfg, reqwest::Method::POST, &format!("/machine/sessions/{id}"), Some(view)).await?;
                                if synced.insert(id) {
                                    self.subscribe_terminal(cfg, &socket_id, &mut sink, id).await?;
                                }
                            }
                            None => {
                                if synced.remove(&id) {
                                    self.drop_session(cfg, id, &mut sink, &mut viewers, &mut forwarders).await?;
                                }
                            }
                        }
                    }
                    self.set_counts(synced.len(), viewers.values().map(HashSet::len).sum());
                }
                _ = heartbeat.tick() => self.heartbeat(cfg).await?,
                _ = ping.tick() => send(&mut sink, json!({"event": "pusher:ping", "data": {}}).to_string()).await?,
                Some(frame) = out_rx.recv() => send(&mut sink, frame).await?,
            }
        };

        for (_, f) in forwarders {
            f.abort();
        }
        result
    }

    /// Sends the full session list and subscribes to each one's terminal channel.
    async fn full_sync(
        &self,
        cfg: &Paired,
        socket_id: &str,
        sink: &mut WsSink,
        synced: &mut HashSet<SessionId>,
    ) -> Result<(), Fail> {
        let views = self.all_payloads().await;
        let ids: HashSet<SessionId> = views
            .iter()
            .filter_map(|v| v.get("id")?.as_str()?.parse::<uuid::Uuid>().ok())
            .map(SessionId)
            .collect();
        self.api(
            cfg,
            reqwest::Method::PUT,
            "/machine/sessions",
            Some(json!({ "sessions": views })),
        )
        .await?;
        for id in ids.difference(synced).copied().collect::<Vec<_>>() {
            self.subscribe_terminal(cfg, socket_id, sink, id).await?;
        }
        for id in synced.difference(&ids).copied().collect::<Vec<_>>() {
            send(sink, unsubscribe_frame(&terminal_channel(id))).await?;
        }
        *synced = ids;
        Ok(())
    }

    async fn subscribe_terminal(
        &self,
        cfg: &Paired,
        socket_id: &str,
        sink: &mut WsSink,
        id: SessionId,
    ) -> Result<(), Fail> {
        let channel = terminal_channel(id);
        let auth = self.channel_auth(cfg, socket_id, &channel).await?;
        send(sink, subscribe_frame(&channel, &auth)).await
    }

    async fn drop_session(
        &self,
        cfg: &Paired,
        id: SessionId,
        sink: &mut WsSink,
        viewers: &mut HashMap<SessionId, HashSet<String>>,
        forwarders: &mut HashMap<SessionId, JoinHandle<()>>,
    ) -> Result<(), Fail> {
        viewers.remove(&id);
        if let Some(f) = forwarders.remove(&id) {
            f.abort();
        }
        send(sink, unsubscribe_frame(&terminal_channel(id))).await?;
        match self
            .api(
                cfg,
                reqwest::Method::DELETE,
                &format!("/machine/sessions/{id}"),
                None,
            )
            .await
        {
            Err(Fail::Other(e)) => {
                tracing::debug!(error = %e, "remote: removing a session failed");
                Ok(())
            }
            other => other.map(|_| ()),
        }
    }

    /// Streams a terminal while someone watches it.
    fn update_forwarder(
        &self,
        id: SessionId,
        viewers: &HashMap<SessionId, HashSet<String>>,
        forwarders: &mut HashMap<SessionId, JoinHandle<()>>,
        out: &mpsc::Sender<String>,
    ) {
        let watched = viewers.get(&id).is_some_and(|v| !v.is_empty());
        match (watched, forwarders.contains_key(&id)) {
            (true, false) => {
                if let Ok(att) = self.tl.attach(id) {
                    forwarders.insert(id, spawn_forwarder(self.tl.clone(), id, att, out.clone()));
                }
            }
            (false, true) => {
                if let Some(f) = forwarders.remove(&id) {
                    f.abort();
                }
            }
            _ => {}
        }
    }

    fn run_command(&self, data: &Value) {
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let Some(id) = data
            .get("session_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            .map(SessionId)
        else {
            return;
        };
        let args = data.get("data").cloned().unwrap_or(Value::Null);
        let tl = self.tl.clone();
        tokio::task::spawn_blocking(move || {
            let result = match kind.as_str() {
                "answer" => {
                    match serde_json::from_value::<ApprovalChoice>(args["choice"].clone()) {
                        Ok(choice) => tl.answer(id, choice).map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    }
                }
                "message" => tl
                    .send_message(id, args["text"].as_str().unwrap_or_default())
                    .map_err(|e| e.to_string()),
                "stop" => tl.stop(id).map_err(|e| e.to_string()),
                "hold" => tl.set_on_hold(id, true).map_err(|e| e.to_string()),
                other => Err(format!("unknown command {other}")),
            };
            match result {
                Ok(()) => tracing::info!(%id, command = %kind, "remote: command applied"),
                Err(e) => {
                    tracing::warn!(%id, command = %kind, error = %e, "remote: command failed")
                }
            }
        });
    }

    // ---- session views -----------------------------------------------------------------

    async fn all_payloads(&self) -> Vec<Value> {
        let tl = self.tl.clone();
        let list = tokio::task::spawn_blocking(move || {
            let folders = folder_map(&tl);
            let now = now_ms();
            let mut list: Vec<SessionView> = tl
                .list_sessions()
                .unwrap_or_default()
                .into_iter()
                .filter(|v| shown_remotely(v, now))
                .collect();
            list.sort_by_key(|v| Reverse(v.record.last_activity_at));
            list.truncate(MAX_SESSIONS);
            (folders, list)
        })
        .await;
        let Ok((folders, list)) = list else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(list.len());
        for v in list {
            if let Some(f) = folders.get(&v.record.folder_id) {
                out.push(self.payload(&v, f).await);
            }
        }
        out
    }

    /// The session's view, or `None` when it shouldn't be shown remotely.
    async fn session_payload(&self, id: SessionId) -> Option<Value> {
        let tl = self.tl.clone();
        let (v, folder) = tokio::task::spawn_blocking(move || {
            let v = tl.session(id).ok()?;
            let folder = folder_map(&tl).remove(&v.record.folder_id)?;
            Some((v, folder))
        })
        .await
        .ok()??;
        if !shown_remotely(&v, now_ms()) {
            return None;
        }
        Some(self.payload(&v, &folder).await)
    }

    async fn payload(&self, v: &SessionView, folder: &Folder) -> Value {
        let branch = self.branch(folder).await;
        let files = if v.live {
            let env = self.tl.env().clone();
            let dir = folder.path.clone();
            let base = v.record.start_commit.clone();
            tokio::task::spawn_blocking(move || {
                base.map(|b| git::changed_files(&env, &dir, &b))
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        let r = &v.record;
        let title = if r.title.trim().is_empty() {
            r.agent.display_name().to_string()
        } else {
            clip(&r.title, 300)
        };
        let approval = v.approval.as_ref().map(|a| {
            json!({
                "kind": if a.kind == PromptKind::Question { "question" } else { "permission" },
                "question": clip(if a.question.trim().is_empty() { "Waiting for your answer" } else { &a.question }, 500),
                "detail": a.detail.as_deref().map(|d| clip(d, 2000)),
                "options": a.options.iter().take(10).map(|o| json!({ "choice": o.choice, "label": clip(&o.label, 200) })).collect::<Vec<_>>(),
            })
        });
        let preview: Vec<String> = v
            .preview
            .iter()
            .rev()
            .take(PREVIEW_LINES)
            .rev()
            .map(|l| clip(l, 500))
            .collect();
        json!({
            "id": r.id.to_string(),
            "project": clip(&folder.name, 200),
            "title": title,
            "agent": r.agent.as_str(),
            "model": v.usage.as_ref().and_then(|u| u.model.clone()).or_else(|| r.model.clone()).map(|m| clip(&m, 100)),
            "branch": branch.map(|b| clip(&b, 200)),
            "status": r.status.as_str(),
            "approval": approval,
            "preview": preview,
            "tokens": v.usage.as_ref().map(|u| u.total_tokens),
            "context_pct": v.context_percent,
            "changed_files": files.iter().take(200).map(|f| json!({
                "path": clip(&f.path, 500),
                "added": f.added.unwrap_or(0),
                "removed": f.deleted.unwrap_or(0),
            })).collect::<Vec<_>>(),
        })
    }

    async fn branch(&self, folder: &Folder) -> Option<String> {
        if let Some((b, at)) = self.branches.lock().expect("branch lock").get(&folder.id)
            && at.elapsed() < BRANCH_TTL
        {
            return b.clone();
        }
        let env = self.tl.env().clone();
        let dir = folder.path.clone();
        let b = tokio::task::spawn_blocking(move || git::branch(&env, &dir))
            .await
            .ok()
            .flatten();
        self.branches
            .lock()
            .expect("branch lock")
            .insert(folder.id, (b.clone(), Instant::now()));
        b
    }

    // ---- pairing -----------------------------------------------------------------------

    async fn do_pair(self: Arc<Self>, req: PairRequest) -> Result<RemoteStatus, String> {
        let server_url = normalize_url(&req.server_url)?;
        let code = req.code.trim();
        if code.is_empty() {
            return Err("Enter the pairing code from your ExampleLab dashboard.".into());
        }
        let name = if req.machine_name.trim().is_empty() {
            self.default_name.clone()
        } else {
            req.machine_name.trim().to_string()
        };
        let res = self
            .http
            .post(format!("{server_url}/api/v1/pair"))
            .header("Accept", "application/json")
            .json(&json!({ "code": code, "name": name, "os": self.os, "version": VERSION }))
            .send()
            .await
            .map_err(|e| format!("Couldn't reach {server_url}: {}", network_error(&e)))?;
        let status = res.status();
        if status.is_redirection() {
            let to = res
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .and_then(|l| reqwest::Url::parse(l).ok())
                .map(|u| u.origin().ascii_serialization());
            return Err(match to {
                Some(origin) if origin != server_url => {
                    format!(
                        "{server_url} redirects to {origin}. Use {origin} as the server address."
                    )
                }
                _ => format!("{server_url} redirected the request. Check the server address."),
            });
        }
        let text = res.text().await.unwrap_or_default();
        if matches!(status.as_u16(), 404 | 405) {
            return Err(format!(
                "{server_url} doesn't look like an ExampleLab server (it answered {}). Check the address; \
                 a server on your own computer usually needs http://.",
                status.as_u16()
            ));
        }
        if !status.is_success() {
            return Err(server_message(status, &text));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|_| format!("{server_url} doesn't look like an ExampleLab server."))?;
        let reverb: Reverb = serde_json::from_value(v["reverb"].clone()).map_err(|_| {
            "The server's reply was missing its live-connection details.".to_string()
        })?;
        let paired = Paired {
            server_url,
            token: v["token"].as_str().unwrap_or_default().to_string(),
            machine_id: v["machine_id"].as_u64().unwrap_or_default(),
            machine_name: name,
            account_name: v
                .pointer("/account/name")
                .and_then(Value::as_str)
                .map(str::to_string),
            plan: v
                .pointer("/account/plan")
                .and_then(Value::as_str)
                .map(str::to_string),
            reverb,
        };
        if paired.token.is_empty() || paired.machine_id == 0 {
            return Err("The server's reply was missing the machine token.".into());
        }
        write_private(
            &self.path,
            &serde_json::to_vec_pretty(&paired).unwrap_or_default(),
        )
        .map_err(|e| format!("Couldn't save the pairing: {e}"))?;
        {
            let mut inner = self.inner.lock().expect("remote lock");
            inner.paired = Some(paired);
            inner.state = RemoteState::Connecting;
            inner.error = None;
        }
        self.start();
        tracing::info!("remote: paired");
        Ok(self.status())
    }

    async fn do_unpair(self: Arc<Self>) -> Result<RemoteStatus, String> {
        let (paired, task) = {
            let mut inner = self.inner.lock().expect("remote lock");
            (inner.paired.clone(), inner.task.take())
        };
        if let Some(t) = task {
            t.abort();
        }
        if let Some(cfg) = paired {
            // Best effort: the dashboard forgets the machine too.
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                self.api(&cfg, reqwest::Method::DELETE, "/machine", None),
            )
            .await;
        }
        self.forget();
        self.set_state(RemoteState::Unpaired, None);
        tracing::info!("remote: unpaired");
        Ok(self.status())
    }
}

/// The bridge as the IPC server sees it.
pub struct RemoteHandle(pub Arc<Bridge>);

impl RemoteControl for RemoteHandle {
    fn status(&self) -> RemoteStatus {
        self.0.status()
    }

    fn pair(&self, request: PairRequest) -> RemoteFuture<RemoteStatus> {
        Box::pin(self.0.clone().do_pair(request))
    }

    fn unpair(&self) -> RemoteFuture<RemoteStatus> {
        Box::pin(self.0.clone().do_unpair())
    }

    fn keeps_alive(&self) -> bool {
        self.0.inner.lock().expect("remote lock").paired.is_some()
    }
}

impl Bridge {
    pub fn status(&self) -> RemoteStatus {
        let inner = self.inner.lock().expect("remote lock");
        let p = inner.paired.as_ref();
        RemoteStatus {
            state: inner.state,
            server_url: p.map(|p| p.server_url.clone()),
            machine_name: p.map(|p| p.machine_name.clone()),
            account_name: p.and_then(|p| p.account_name.clone()),
            plan: p.and_then(|p| p.plan.clone()),
            error: inner.error.clone(),
            sessions: inner.sessions,
            viewers: inner.viewers,
            default_machine_name: self.default_name.clone(),
        }
    }
}

// ---- terminal streaming ------------------------------------------------------------

/// Sends a snapshot, then live output, as client events on the session's channel.
fn spawn_forwarder(
    tl: Arc<Termlab>,
    id: SessionId,
    att: termlab_core::pty::Attachment,
    out: mpsc::Sender<String>,
) -> JoinHandle<()> {
    let channel = terminal_channel(id);
    tokio::spawn(async move {
        let mut events = att.events;
        if !send_snapshot(&out, &channel, att.size, &att.snapshot).await {
            return;
        }
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let mut done = false;
            match events.recv().await {
                Ok(PtyEvent::Output(b)) => pending.extend_from_slice(&b),
                Ok(PtyEvent::Exited(_)) | Err(broadcast::error::RecvError::Closed) => done = true,
                Err(broadcast::error::RecvError::Lagged(_)) => match tl.attach(id) {
                    Ok(fresh) => {
                        pending.clear();
                        events = fresh.events;
                        if !send_snapshot(&out, &channel, fresh.size, &fresh.snapshot).await {
                            return;
                        }
                        continue;
                    }
                    Err(_) => done = true,
                },
            }
            if !done {
                let deadline = tokio::time::Instant::now() + OUTPUT_BATCH;
                while pending.len() < CHUNK * 8 {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => break,
                        ev = events.recv() => match ev {
                            Ok(PtyEvent::Output(b)) => pending.extend_from_slice(&b),
                            Ok(PtyEvent::Exited(_)) | Err(broadcast::error::RecvError::Closed) => { done = true; break }
                            Err(broadcast::error::RecvError::Lagged(_)) => break,
                        }
                    }
                }
            }
            for chunk in pending.chunks(CHUNK) {
                let frame = client_event(
                    &channel,
                    "client-output",
                    json!({ "data": B64.encode(chunk) }),
                );
                if out.send(frame).await.is_err() {
                    return;
                }
            }
            pending.clear();
            if done {
                return;
            }
        }
    })
}

async fn send_snapshot(
    out: &mpsc::Sender<String>,
    channel: &str,
    size: Option<termlab_core::model::TermSize>,
    snapshot: &[u8],
) -> bool {
    // Only the recent part: a viewer doesn't need megabytes of history. Cut at a line start.
    let mut tail = snapshot;
    if tail.len() > SNAPSHOT_MAX {
        tail = &tail[tail.len() - SNAPSHOT_MAX..];
        if let Some(nl) = tail.iter().position(|b| *b == b'\n') {
            tail = &tail[nl + 1..];
        }
    }
    let mut bytes = replay_prefix(size);
    bytes.extend_from_slice(tail);
    for (i, chunk) in bytes.chunks(CHUNK).enumerate() {
        let frame = if i == 0 {
            client_event(
                channel,
                "client-snapshot",
                json!({ "data": B64.encode(chunk), "cols": size.map(|s| s.cols), "rows": size.map(|s| s.rows) }),
            )
        } else {
            client_event(
                channel,
                "client-output",
                json!({ "data": B64.encode(chunk) }),
            )
        };
        if out.send(frame).await.is_err() {
            return false;
        }
    }
    true
}

// ---- Pusher protocol helpers -------------------------------------------------------

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

async fn send(sink: &mut WsSink, frame: String) -> Result<(), Fail> {
    sink.send(Message::Text(frame.into()))
        .await
        .map_err(|e| Fail::Other(format!("Live connection lost: {e}")))
}

/// `(event, channel, data)`; `data` is decoded when the server sent it as a JSON string.
fn parse_frame(text: &str) -> (String, String, Value) {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let event = v
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let channel = v
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let data = match v.get("data") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
        Some(other) => other.clone(),
        None => Value::Null,
    };
    (event, channel, data)
}

fn subscribe_frame(channel: &str, auth: &Value) -> String {
    let mut data =
        json!({ "channel": channel, "auth": auth.get("auth").cloned().unwrap_or(Value::Null) });
    if let Some(cd) = auth.get("channel_data") {
        data["channel_data"] = cd.clone();
    }
    json!({ "event": "pusher:subscribe", "data": data }).to_string()
}

fn unsubscribe_frame(channel: &str) -> String {
    json!({ "event": "pusher:unsubscribe", "data": { "channel": channel } }).to_string()
}

fn client_event(channel: &str, event: &str, data: Value) -> String {
    json!({ "event": event, "channel": channel, "data": data }).to_string()
}

fn terminal_channel(id: SessionId) -> String {
    format!("presence-terminal.{id}")
}

fn terminal_session(channel: &str) -> Option<SessionId> {
    channel
        .strip_prefix("presence-terminal.")?
        .parse::<uuid::Uuid>()
        .ok()
        .map(SessionId)
}

fn member_id(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// ---- small helpers -----------------------------------------------------------------

fn shown_remotely(v: &SessionView, now: i64) -> bool {
    let r = &v.record;
    !r.on_hold
        && (v.live || (r.origin != SessionOrigin::Imported && now - r.last_activity_at < RECENT_MS))
}

fn folder_map(tl: &Termlab) -> HashMap<FolderId, Folder> {
    tl.list_folders()
        .unwrap_or_default()
        .into_iter()
        .map(|f| (f.id, f))
        .collect()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars()
            .take(max.saturating_sub(1))
            .chain(std::iter::once('…'))
            .collect()
    }
}

fn normalize_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err("Enter the address of your ExampleLab server.".into());
    }
    let url = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let parsed = reqwest::Url::parse(&url).map_err(|_| format!("{raw} isn't a valid address."))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(format!("{raw} isn't a valid address."));
    }
    Ok(url)
}

/// The server's `message`, or a plain description of the status.
fn server_message(status: reqwest::StatusCode, body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("message")?.as_str().map(str::to_string))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("The server answered {status}."))
}

fn network_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "the server didn't answer in time".into()
    } else if e.is_connect() {
        "couldn't connect".into()
    } else {
        e.to_string()
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, path)
}

fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn default_machine_name() -> String {
    #[cfg(target_os = "macos")]
    if let Some(n) = command_output("scutil", &["--get", "ComputerName"]) {
        return n;
    }
    #[cfg(windows)]
    if let Ok(n) = std::env::var("COMPUTERNAME") {
        return n;
    }
    command_output("hostname", &[])
        .map(|h| h.split('.').next().unwrap_or(&h).to_string())
        .unwrap_or_else(|| "This computer".into())
}

fn os_description() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" if cfg!(target_os = "macos") => "Apple Silicon",
        "aarch64" => "ARM64",
        "x86_64" => "x86-64",
        other => other,
    };
    #[cfg(target_os = "macos")]
    let os = command_output("sw_vers", &["-productVersion"])
        .map(|v| format!("macOS {v}"))
        .unwrap_or_else(|| "macOS".into());
    #[cfg(target_os = "linux")]
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "Linux".into());
    #[cfg(windows)]
    let os = "Windows".to_string();
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    let os = std::env::consts::OS.to_string();
    format!("{os} · {arch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_decode_string_data() {
        let (event, channel, data) = parse_frame(
            r#"{"event":"command","channel":"private-machine.3","data":"{\"type\":\"stop\",\"session_id\":\"x\"}"}"#,
        );
        assert_eq!(event, "command");
        assert_eq!(channel, "private-machine.3");
        assert_eq!(data["type"], "stop");
    }

    #[test]
    fn terminal_channels_round_trip() {
        let id = SessionId(uuid::Uuid::new_v4());
        assert_eq!(terminal_session(&terminal_channel(id)), Some(id));
        assert_eq!(terminal_session("private-machine.1"), None);
    }

    #[test]
    fn urls_are_normalized() {
        assert_eq!(
            normalize_url("examplelab.com/").unwrap(),
            "https://examplelab.com"
        );
        assert_eq!(
            normalize_url("http://examplelab.test").unwrap(),
            "http://examplelab.test"
        );
        assert!(normalize_url("").is_err());
        assert!(normalize_url("ftp://x").is_err());
    }

    #[test]
    fn subscribe_includes_presence_data() {
        let f = subscribe_frame(
            "presence-terminal.x",
            &json!({"auth": "k:sig", "channel_data": "{\"user_id\":\"machine-1\"}"}),
        );
        let v: Value = serde_json::from_str(&f).unwrap();
        assert_eq!(v["data"]["auth"], "k:sig");
        assert_eq!(v["data"]["channel_data"], "{\"user_id\":\"machine-1\"}");
    }

    #[test]
    fn server_choices_parse_as_approval_choices() {
        for (wire, choice) in [
            ("allow_once", ApprovalChoice::AllowOnce),
            ("allow_always", ApprovalChoice::AllowAlways),
            ("deny", ApprovalChoice::Deny),
        ] {
            assert_eq!(
                serde_json::from_value::<ApprovalChoice>(json!(wire)).unwrap(),
                choice
            );
        }
    }

    #[test]
    fn clip_keeps_short_text() {
        assert_eq!(clip("abc", 5), "abc");
        assert_eq!(clip("abcdef", 4), "abc…");
    }
}
