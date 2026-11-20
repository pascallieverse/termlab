//! The remote-access bridge: links this computer's sessions to your own Termlab server
//! (`termlab-server`, self-hosted).
//!
//! One websocket to `/api/machine/ws` carries everything:
//! - session state goes up (a snapshot, then each change, debounced); the server diffs it
//!   and fires webhooks and phone notifications;
//! - commands (answer, message, stop, hold) come down, audited by the server first;
//! - a session's terminal streams only while someone watches it in a browser, and their
//!   keystrokes come back.
//!
//! The protocol is in `docs/server-protocol.md`.

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
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const CONFIG_FILE: &str = "remote.json";
/// The server protocol this bridge speaks (docs/server-protocol.md).
const PROTOCOL: u32 = 2;
const PING: Duration = Duration::from_secs(25);
/// A session that changes often is reported at most this often.
const DEBOUNCE: Duration = Duration::from_millis(300);
const FLUSH_TICK: Duration = Duration::from_millis(150);
/// Sessions shown remotely: live ones, plus recent ones that finished.
const RECENT_MS: i64 = 48 * 3600 * 1000;
const MAX_SESSIONS: usize = 200;
const PREVIEW_LINES: usize = 12;
/// Raw terminal bytes per output message (the server accepts up to 256 KB of base64).
const CHUNK: usize = 48 * 1024;
const OUTPUT_BATCH: Duration = Duration::from_millis(30);
/// How much scrollback a viewer gets when they open a terminal.
const SNAPSHOT_MAX: usize = 512 * 1024;
const BRANCH_TTL: Duration = Duration::from_secs(60);
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What pairing returned; kept in `<data-dir>/remote.json` (mode 0600).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Paired {
    #[serde(default)]
    protocol: u32,
    server_url: String,
    token: String,
    machine_id: u64,
    machine_name: String,
    server_name: Option<String>,
}

enum Fail {
    /// The token was revoked: the computer was removed on the server.
    Revoked,
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
            .and_then(|b| serde_json::from_slice::<Paired>(&b).ok())
            // A pairing from before protocol 2 (the hosted ExampleLab service) is dropped.
            .filter(|p| p.protocol == PROTOCOL);
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
                        Some("This computer was removed on your Termlab server.".into()),
                    );
                    return;
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

    // ---- one connection ----------------------------------------------------------------

    async fn connect_once(self: &Arc<Self>, cfg: &Paired) -> Result<(), Fail> {
        let url = ws_url(&cfg.server_url, "/api/machine/ws").ok_or_else(|| {
            Fail::Other(format!("{} isn't a valid server address.", cfg.server_url))
        })?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| Fail::Other(format!("Bad server address: {e}")))?;
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", cfg.token)
                .parse()
                .map_err(|_| Fail::Other("Bad machine token.".into()))?,
        );
        request.headers_mut().insert(
            "user-agent",
            format!("Termlab/{VERSION}").parse().expect("ascii"),
        );
        let connector = if url.scheme() == "wss" {
            use rustls_platform_verifier::ConfigVerifierExt;
            let tls = rustls::ClientConfig::with_platform_verifier()
                .map_err(|e| Fail::Other(format!("TLS setup failed: {e}")))?;
            Some(tokio_tungstenite::Connector::Rustls(Arc::new(tls)))
        } else {
            None
        };
        let (ws, _) =
            tokio_tungstenite::connect_async_tls_with_config(request, None, false, connector)
                .await
                .map_err(|e| match e {
                    tokio_tungstenite::tungstenite::Error::Http(res)
                        if res.status().as_u16() == 401 =>
                    {
                        Fail::Revoked
                    }
                    tokio_tungstenite::tungstenite::Error::Http(res) => Fail::Other(format!(
                        "The server refused the connection ({}).",
                        res.status()
                    )),
                    other => Fail::Other(format!("Couldn't reach the server: {other}")),
                })?;
        let (mut sink, mut stream) = ws.split();

        send(
            &mut sink,
            json!({ "type": "hello", "protocol": PROTOCOL, "version": VERSION, "os": self.os }),
        )
        .await?;
        let mut synced: HashSet<SessionId> = HashSet::new();
        self.full_sync(&mut sink, &mut synced).await?;
        self.set_state(RemoteState::Connected, None);
        self.set_counts(synced.len(), 0);
        tracing::info!(sessions = synced.len(), "remote: connected");

        let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
        let mut core_rx = self.tl.subscribe();
        let mut forwarders: HashMap<SessionId, JoinHandle<()>> = HashMap::new();
        let mut dirty: HashMap<SessionId, Instant> = HashMap::new();
        let mut flush = tokio::time::interval(FLUSH_TICK);
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
                        Some(Err(e)) => break Err(Fail::Other(format!("Connection lost: {e}"))),
                    };
                    let Ok(m) = serde_json::from_str::<Value>(&text) else { continue };
                    let id = m["id"].as_str().and_then(|s| s.parse::<uuid::Uuid>().ok()).map(SessionId);
                    match m["type"].as_str().unwrap_or("") {
                        "command" => self.run_command(&m),
                        "watch" => if let Some(id) = id {
                            if m["on"].as_bool().unwrap_or(false) {
                                // A (new) viewer: start over with a fresh snapshot.
                                if let Some(f) = forwarders.remove(&id) { f.abort(); }
                                if let Ok(att) = self.tl.attach(id) {
                                    forwarders.insert(id, spawn_forwarder(self.tl.clone(), id, att, out_tx.clone()));
                                }
                            } else if let Some(f) = forwarders.remove(&id) {
                                f.abort();
                            }
                            self.set_counts(synced.len(), forwarders.len());
                        },
                        "input" => if let (Some(id), Some(bytes)) = (id, m["data"].as_str().and_then(|d| B64.decode(d).ok())) {
                            let _ = self.tl.write(id, bytes);
                        },
                        "bye" => break Err(Fail::Revoked),
                        _ => {}
                    }
                }
                ev = core_rx.recv() => match ev {
                    Ok(CoreEvent::SessionUpdated { session }) => { dirty.entry(session.record.id).or_insert_with(Instant::now); }
                    Ok(CoreEvent::SessionRemoved { id }) => {
                        dirty.remove(&id);
                        if let Some(f) = forwarders.remove(&id) { f.abort(); }
                        if synced.remove(&id) {
                            send(&mut sink, json!({ "type": "removed", "id": id.to_string() })).await?;
                            self.set_counts(synced.len(), forwarders.len());
                        }
                    }
                    Ok(CoreEvent::FoldersChanged) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        dirty.clear();
                        self.branches.lock().expect("branch lock").clear();
                        self.full_sync(&mut sink, &mut synced).await?;
                        self.set_counts(synced.len(), forwarders.len());
                    }
                    Err(broadcast::error::RecvError::Closed) => break Ok(()),
                },
                _ = flush.tick() => {
                    let due: Vec<SessionId> = dirty.iter().filter(|(_, t)| t.elapsed() >= DEBOUNCE).map(|(id, _)| *id).collect();
                    for id in due {
                        dirty.remove(&id);
                        match self.session_payload(id).await {
                            Some(view) => {
                                send(&mut sink, json!({ "type": "session", "session": view })).await?;
                                synced.insert(id);
                            }
                            None => {
                                if synced.remove(&id) {
                                    send(&mut sink, json!({ "type": "removed", "id": id.to_string() })).await?;
                                }
                            }
                        }
                    }
                    self.set_counts(synced.len(), forwarders.len());
                }
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break Err(Fail::Other("Connection lost.".into()));
                    }
                }
                Some(frame) = out_rx.recv() => {
                    if sink.send(Message::Text(frame.into())).await.is_err() {
                        break Err(Fail::Other("Connection lost.".into()));
                    }
                }
            }
        };

        for (_, f) in forwarders {
            f.abort();
        }
        result
    }

    /// Sends the full session list.
    async fn full_sync(
        &self,
        sink: &mut WsSink,
        synced: &mut HashSet<SessionId>,
    ) -> Result<(), Fail> {
        let views = self.all_payloads().await;
        *synced = views
            .iter()
            .filter_map(|v| v.get("id")?.as_str()?.parse::<uuid::Uuid>().ok())
            .map(SessionId)
            .collect();
        send(sink, json!({ "type": "snapshot", "sessions": views })).await
    }

    fn run_command(&self, data: &Value) {
        let kind = data
            .get("command")
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
            return Err(
                "Enter the pairing code from your Termlab server (Machines › Pair a machine)."
                    .into(),
            );
        }
        let name = if req.machine_name.trim().is_empty() {
            self.default_name.clone()
        } else {
            req.machine_name.trim().to_string()
        };
        let res = self
            .http
            .post(format!("{server_url}/api/pair"))
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
                "{server_url} doesn't look like a Termlab server (it answered {}). Check the address; \
                 a server on your own computer usually needs http://.",
                status.as_u16()
            ));
        }
        if !status.is_success() {
            return Err(server_message(status, &text));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|_| format!("{server_url} doesn't look like a Termlab server."))?;
        let paired = Paired {
            protocol: PROTOCOL,
            server_url,
            token: v["token"].as_str().unwrap_or_default().to_string(),
            machine_id: v["machine_id"].as_u64().unwrap_or_default(),
            machine_name: name,
            server_name: v["server_name"].as_str().map(str::to_string),
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
            // Best effort: the server forgets this computer too.
            let sent = tokio::time::timeout(
                Duration::from_secs(5),
                self.http
                    .delete(format!("{}/api/machine", cfg.server_url))
                    .bearer_auth(&cfg.token)
                    .send(),
            )
            .await;
            match sent {
                Ok(Ok(r)) if r.status().is_success() || r.status().as_u16() == 401 => {}
                Ok(Ok(r)) => {
                    tracing::warn!(status = %r.status(), "remote: the server refused the unlink")
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "remote: couldn't reach the server to unlink")
                }
                Err(_) => tracing::warn!("remote: the server didn't answer the unlink in time"),
            }
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
            account_name: p.and_then(|p| p.server_name.clone()),
            plan: None,
            error: inner.error.clone(),
            sessions: inner.sessions,
            viewers: inner.viewers,
            default_machine_name: self.default_name.clone(),
        }
    }
}

// ---- terminal streaming ------------------------------------------------------------

/// Sends a snapshot, then live output, while someone watches.
fn spawn_forwarder(
    tl: Arc<Termlab>,
    id: SessionId,
    att: termlab_core::pty::Attachment,
    out: mpsc::Sender<String>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut events = att.events;
        if !send_snapshot(&out, id, att.size, &att.snapshot).await {
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
                        if !send_snapshot(&out, id, fresh.size, &fresh.snapshot).await {
                            return;
                        }
                        continue;
                    }
                    Err(_) => done = true,
                },
            }
            if !done {
                let deadline = tokio::time::Instant::now() + OUTPUT_BATCH;
                while pending.len() < CHUNK * 4 {
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
                if out
                    .send(output_frame(id, chunk, false, None))
                    .await
                    .is_err()
                {
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
    id: SessionId,
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
            output_frame(id, chunk, true, size)
        } else {
            output_frame(id, chunk, false, None)
        };
        if out.send(frame).await.is_err() {
            return false;
        }
    }
    true
}

fn output_frame(
    id: SessionId,
    data: &[u8],
    snapshot: bool,
    size: Option<termlab_core::model::TermSize>,
) -> String {
    json!({
        "type": "output",
        "id": id.to_string(),
        "data": B64.encode(data),
        "snapshot": snapshot,
        "cols": size.map(|s| s.cols),
        "rows": size.map(|s| s.rows),
    })
    .to_string()
}

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

async fn send(sink: &mut WsSink, v: Value) -> Result<(), Fail> {
    sink.send(Message::Text(v.to_string().into()))
        .await
        .map_err(|e| Fail::Other(format!("Connection lost: {e}")))
}

/// `https://host/x` → `wss://host/api/...` (http → ws).
fn ws_url(server_url: &str, path: &str) -> Option<reqwest::Url> {
    let mut u = reqwest::Url::parse(server_url).ok()?;
    let scheme = match u.scheme() {
        "https" => "wss",
        "http" => "ws",
        _ => return None,
    };
    u.set_scheme(scheme).ok()?;
    u.set_path(path);
    u.set_query(None);
    Some(u)
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
        return Err("Enter the address of your Termlab server.".into());
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
    fn websocket_urls_follow_the_server_scheme() {
        assert_eq!(
            ws_url("https://remote.example.com", "/api/machine/ws")
                .unwrap()
                .as_str(),
            "wss://remote.example.com/api/machine/ws"
        );
        assert_eq!(
            ws_url("http://127.0.0.1:8484", "/api/machine/ws")
                .unwrap()
                .as_str(),
            "ws://127.0.0.1:8484/api/machine/ws"
        );
        assert!(ws_url("ftp://x", "/a").is_none());
    }

    #[test]
    fn urls_are_normalized() {
        assert_eq!(
            normalize_url("remote.example.com/").unwrap(),
            "https://remote.example.com"
        );
        assert_eq!(
            normalize_url("http://127.0.0.1:8484").unwrap(),
            "http://127.0.0.1:8484"
        );
        assert!(normalize_url("").is_err());
        assert!(normalize_url("ftp://x").is_err());
    }

    #[test]
    fn output_frames_carry_size_only_on_snapshots() {
        let id = SessionId(uuid::Uuid::new_v4());
        let size = Some(termlab_core::model::TermSize { cols: 90, rows: 24 });
        let snap: Value = serde_json::from_str(&output_frame(id, b"hi", true, size)).unwrap();
        assert_eq!(snap["type"], "output");
        assert_eq!(snap["snapshot"], true);
        assert_eq!(snap["cols"], 90);
        assert_eq!(B64.decode(snap["data"].as_str().unwrap()).unwrap(), b"hi");
        let live: Value = serde_json::from_str(&output_frame(id, b"x", false, None)).unwrap();
        assert!(live["cols"].is_null());
    }

    #[test]
    fn old_pairings_are_not_reused() {
        let old = r#"{"server_url":"https://examplelab.com","token":"t","machine_id":1,"machine_name":"m","account_name":"a","plan":"remote","reverb":{}}"#;
        let p: Paired = serde_json::from_str(old).unwrap();
        assert_ne!(p.protocol, PROTOCOL);
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
