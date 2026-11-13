//! The protocol between the desktop app and `termlabd`, over a Unix socket.
//!
//! Frames: `u32` big-endian length of what follows, then a kind byte, then the payload.
//! - kind 1, JSON: [`ClientMsg`] from the app, [`ServerMsg`] from the daemon.
//! - kind 2, terminal output: 16-byte session id, then raw pty bytes (daemon → app).
//!
//! The app sends `Attach` to receive a session's output: first a full-reset escape plus
//! the scrollback snapshot, then live output batched every 16 ms.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uuid::Uuid;

use crate::adapter::PastSession;
use crate::error::CoreError;
use crate::model::{AgentId, ApprovalChoice, FolderId, PermissionPreset, SessionId, TermSize};
use crate::pty::PtyEvent;
use crate::service::{CoreEvent, StartRequest, Termlab};

/// 2: discovery everywhere, import, external sessions and attach.
pub const PROTOCOL_VERSION: u32 = 2;
/// A call with no reply in this long fails instead of hanging the caller.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const KIND_JSON: u8 = 1;
const KIND_OUTPUT: u8 = 2;
const MAX_FRAME: usize = 64 * 1024 * 1024;
/// Output is sent at most once per frame.
const BATCH_WINDOW: Duration = Duration::from_millis(16);
const MAX_BATCH: usize = 256 * 1024;
/// "Full reset" escape: clears the viewer's terminal before a snapshot is replayed.
pub const RESET: &[u8] = b"\x1bc";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum Request {
    Hello {
        protocol: u32,
    },
    ListFolders,
    MoveFolder {
        id: FolderId,
        before: Option<FolderId>,
    },
    AddFolder {
        path: String,
    },
    #[serde(rename_all = "camelCase")]
    RemoveFolder {
        id: FolderId,
    },
    Agents,
    RefreshAgents,
    /// Each agent's subscription plan limits, as last reported.
    PlanUsage,
    /// Asks the agents for fresh limits in the background; answers with the current ones.
    RefreshPlanUsage,
    ListSessions,
    Session {
        id: SessionId,
    },
    PreviewCommand {
        request: StartRequest,
        id: SessionId,
    },
    StartSession {
        request: StartRequest,
        id: Option<SessionId>,
    },
    Restart {
        id: SessionId,
        size: Option<TermSize>,
    },
    Fork {
        id: SessionId,
        size: Option<TermSize>,
    },
    HandoffMessage {
        id: SessionId,
    },
    HandOff {
        id: SessionId,
        agent: AgentId,
        permission: PermissionPreset,
        model: Option<String>,
        size: Option<TermSize>,
    },
    Attach {
        id: SessionId,
    },
    Detach {
        id: SessionId,
    },
    Write {
        id: SessionId,
        data: String,
    },
    WriteBytes {
        id: SessionId,
        data: Vec<u8>,
    },
    SendMessage {
        id: SessionId,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Broadcast {
        folder_id: FolderId,
        text: String,
    },
    Answer {
        id: SessionId,
        choice: ApprovalChoice,
    },
    Resize {
        id: SessionId,
        cols: u16,
        rows: u16,
    },
    Stop {
        id: SessionId,
    },
    #[serde(rename_all = "camelCase")]
    StopIdle {
        folder_id: FolderId,
    },
    Delete {
        id: SessionId,
    },
    SetTitle {
        id: SessionId,
        title: String,
    },
    Details {
        id: SessionId,
    },
    Diff {
        id: SessionId,
    },
    Timeline {
        id: SessionId,
    },
    /// What one of the session's sub-agents did.
    SubagentTimeline {
        id: SessionId,
        subagent: String,
    },
    #[serde(rename_all = "camelCase")]
    Discover {
        folder_id: Option<FolderId>,
        #[serde(default)]
        everywhere: bool,
    },
    /// Agent sessions running right now outside Termlab.
    External,
    /// Adds past agent sessions to Termlab.
    Import {
        items: Vec<PastSession>,
    },
    #[serde(rename_all = "camelCase")]
    AttachExternal {
        agent: AgentId,
        agent_session_id: String,
        size: Option<TermSize>,
    },
    /// Stops the daemon (only honoured when no sessions are running, unless `force`).
    Shutdown {
        force: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMsg {
    pub id: u64,
    pub req: Request,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMsg {
    Reply {
        id: u64,
        ok: Option<Value>,
        err: Option<String>,
    },
    Event {
        event: CoreEvent,
    },
    /// The app missed events and should reload its state.
    Resync,
}

/// What `Hello` returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonInfo {
    pub protocol: u32,
    pub version: String,
    pub pid: u32,
    pub started_at: i64,
    pub live_sessions: usize,
}

// ---- framing --------------------------------------------------------------------

async fn write_frame<W: AsyncWriteExt + Unpin>(
    w: &mut W,
    kind: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = u32::try_from(payload.len() + 1)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame too large"))?;
    let mut head = [0u8; 5];
    head[..4].copy_from_slice(&len.to_be_bytes());
    head[4] = kind;
    w.write_all(&head).await?;
    w.write_all(payload).await?;
    w.flush().await
}

async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> std::io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad frame length",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    let kind = buf[0];
    buf.remove(0);
    Ok((kind, buf))
}

fn json_frame<T: Serialize>(msg: &T) -> (u8, Vec<u8>) {
    (KIND_JSON, serde_json::to_vec(msg).unwrap_or_default())
}

fn output_frame(id: SessionId, data: &[u8]) -> (u8, Vec<u8>) {
    let mut payload = Vec::with_capacity(16 + data.len());
    payload.extend_from_slice(id.0.as_bytes());
    payload.extend_from_slice(data);
    (KIND_OUTPUT, payload)
}

// ---- server -----------------------------------------------------------------------

/// Daemon-side state shared by connections.
pub struct Server {
    pub tl: Arc<Termlab>,
    pub started_at: i64,
    pub clients: AtomicUsize,
    /// Fired when a client asks the daemon to stop.
    pub shutdown: tokio::sync::Notify,
}

impl Server {
    pub fn new(tl: Arc<Termlab>) -> Arc<Self> {
        Arc::new(Self {
            tl,
            started_at: crate::model::now_ms(),
            clients: AtomicUsize::new(0),
            shutdown: tokio::sync::Notify::new(),
        })
    }

    /// Accepts connections on `path` until the process exits.
    pub async fn serve(self: Arc<Self>, path: &Path) -> std::io::Result<()> {
        let _ = std::fs::remove_file(path);
        let listener = tokio::net::UnixListener::bind(path)?;
        crate::service::restrict_permissions(path);
        loop {
            let (stream, _) = listener.accept().await?;
            let this = self.clone();
            tokio::spawn(async move {
                this.clients.fetch_add(1, Ordering::SeqCst);
                this.clone().handle(stream).await;
                this.clients.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }

    async fn handle(self: Arc<Self>, stream: UnixStream) {
        let (mut reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::channel::<(u8, Vec<u8>)>(1024);

        let writer_task = tokio::spawn(async move {
            while let Some((kind, payload)) = rx.recv().await {
                if write_frame(&mut writer, kind, &payload).await.is_err() {
                    break;
                }
            }
        });

        let events_task = {
            let tx = tx.clone();
            let mut events = self.tl.subscribe();
            tokio::spawn(async move {
                loop {
                    let msg = match events.recv().await {
                        Ok(event) => ServerMsg::Event { event },
                        Err(broadcast::error::RecvError::Lagged(_)) => ServerMsg::Resync,
                        Err(broadcast::error::RecvError::Closed) => break,
                    };
                    if tx.send(json_frame(&msg)).await.is_err() {
                        break;
                    }
                }
            })
        };

        let mut attachments: HashMap<SessionId, JoinHandle<()>> = HashMap::new();
        while let Ok((kind, payload)) = read_frame(&mut reader).await {
            if kind != KIND_JSON {
                continue;
            }
            let Ok(ClientMsg { id, req }) = serde_json::from_slice::<ClientMsg>(&payload) else {
                // Most likely a request from a newer app. Answer it so the caller doesn't wait.
                if let Some(id) = serde_json::from_slice::<Value>(&payload)
                    .ok()
                    .and_then(|v| v.get("id")?.as_u64())
                {
                    let msg = "Termlab's background service is older than the app and doesn't \
                               support this. Restart it from the banner once its sessions are done.";
                    let _ = tx.send(json_frame(&reply_msg(id, Err(msg.into())))).await;
                }
                tracing::debug!("unsupported client message");
                continue;
            };
            match req {
                Request::Attach { id: sid } => {
                    let reply = match self.tl.attach(sid) {
                        Ok(att) => {
                            let exited = att.exited;
                            let task = spawn_forwarder(self.tl.clone(), sid, att, tx.clone());
                            if let Some(old) = attachments.insert(sid, task) {
                                old.abort();
                            }
                            Ok(json!({ "exited": exited }))
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    let _ = tx.send(json_frame(&reply_msg(id, reply))).await;
                }
                Request::Detach { id: sid } => {
                    if let Some(t) = attachments.remove(&sid) {
                        t.abort();
                    }
                    let _ = tx.send(json_frame(&reply_msg(id, Ok(Value::Null)))).await;
                }
                Request::Shutdown { force } => {
                    let live = self.tl.live_count();
                    let reply = if live == 0 || force {
                        self.shutdown.notify_one();
                        Ok(Value::Null)
                    } else {
                        Err(format!("{live} sessions are running"))
                    };
                    let _ = tx.send(json_frame(&reply_msg(id, reply))).await;
                }
                Request::Hello { .. } => {
                    let info = DaemonInfo {
                        protocol: PROTOCOL_VERSION,
                        version: env!("CARGO_PKG_VERSION").into(),
                        pid: std::process::id(),
                        started_at: self.started_at,
                        live_sessions: self.tl.live_count(),
                    };
                    let reply = serde_json::to_value(info).map_err(|e| e.to_string());
                    let _ = tx.send(json_frame(&reply_msg(id, reply))).await;
                }
                // Keystrokes and resizes are handled in order, inline (they don't block).
                req @ (Request::Write { .. }
                | Request::WriteBytes { .. }
                | Request::Resize { .. }
                | Request::SendMessage { .. }
                | Request::Answer { .. }) => {
                    let reply = dispatch(&self.tl, req);
                    let _ = tx.send(json_frame(&reply_msg(id, reply))).await;
                }
                req => {
                    let tl = self.tl.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let reply = tokio::task::spawn_blocking(move || dispatch(&tl, req))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()));
                        let _ = tx.send(json_frame(&reply_msg(id, reply))).await;
                    });
                }
            }
        }
        for (_, t) in attachments {
            t.abort();
        }
        events_task.abort();
        drop(tx);
        let _ = writer_task.await;
    }
}

fn reply_msg(id: u64, r: Result<Value, String>) -> ServerMsg {
    match r {
        Ok(v) => ServerMsg::Reply {
            id,
            ok: Some(v),
            err: None,
        },
        Err(e) => ServerMsg::Reply {
            id,
            ok: None,
            err: Some(e),
        },
    }
}

fn to_value<T: Serialize>(r: crate::Result<T>) -> Result<Value, String> {
    r.map_err(|e| e.to_string())
        .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

/// Runs one request against the service.
pub fn dispatch(tl: &Arc<Termlab>, req: Request) -> Result<Value, String> {
    use Request::*;
    match req {
        ListFolders => to_value(tl.list_folders()),
        MoveFolder { id, before } => to_value(tl.move_folder(id, before)),
        AddFolder { path } => to_value(tl.add_folder(Path::new(&path))),
        RemoveFolder { id } => to_value(tl.remove_folder(id)),
        Agents => to_value(Ok::<_, CoreError>(tl.agents())),
        PlanUsage => to_value(Ok::<_, CoreError>(tl.plan_usage())),
        RefreshPlanUsage => {
            // Asking can start an agent's CLI: don't hold up the connection for it.
            let tl2 = Arc::clone(tl);
            std::thread::spawn(move || tl2.refresh_plan_usage(true));
            to_value(Ok::<_, CoreError>(tl.plan_usage()))
        }
        RefreshAgents => {
            tl.refresh_agents();
            to_value(Ok::<_, CoreError>(tl.agents()))
        }
        ListSessions => to_value(tl.list_sessions()),
        Session { id } => to_value(tl.session(id)),
        PreviewCommand { request, id } => to_value(tl.preview_command(&request, id)),
        StartSession { request, id } => to_value(tl.start_session(&request, id)),
        Restart { id, size } => to_value(tl.restart(id, size)),
        Fork { id, size } => to_value(tl.fork(id, size)),
        HandoffMessage { id } => to_value(tl.handoff_message(id)),
        HandOff {
            id,
            agent,
            permission,
            model,
            size,
        } => to_value(tl.hand_off(id, agent, permission, model, size)),
        Write { id, data } => to_value(tl.write(id, data.into_bytes())),
        WriteBytes { id, data } => to_value(tl.write(id, data)),
        SendMessage { id, text } => to_value(tl.send_message(id, &text)),
        Broadcast { folder_id, text } => to_value(tl.broadcast(folder_id, &text)),
        Answer { id, choice } => to_value(tl.answer(id, choice)),
        Resize { id, cols, rows } => to_value(tl.resize(id, TermSize { cols, rows })),
        Stop { id } => to_value(tl.stop(id)),
        StopIdle { folder_id } => to_value(tl.stop_idle(folder_id)),
        Delete { id } => to_value(tl.delete_session(id)),
        SetTitle { id, title } => to_value(tl.set_title(id, &title)),
        Details { id } => to_value(tl.details(id)),
        Diff { id } => to_value(tl.diff(id)),
        Timeline { id } => to_value(tl.timeline(id)),
        SubagentTimeline { id, subagent } => to_value(tl.subagent_timeline(id, &subagent)),
        Discover {
            folder_id,
            everywhere,
        } => to_value(tl.discover(folder_id, everywhere)),
        External => to_value(Ok::<_, CoreError>(tl.external_sessions())),
        Import { items } => to_value(tl.import(&items)),
        AttachExternal {
            agent,
            agent_session_id,
            size,
        } => to_value(tl.attach_external(agent, &agent_session_id, size)),
        Hello { .. } | Attach { .. } | Detach { .. } | Shutdown { .. } => {
            Err("handled by the connection".into())
        }
    }
}

/// What precedes a replayed snapshot: a full reset, then (when known) the size the output
/// was drawn at, as the standard xterm "resize to rows;cols" sequence (`CSI 8 ; r ; c t`).
/// Viewers must replay at that size; TUI output replayed at another size comes out garbled.
pub fn replay_prefix(size: Option<TermSize>) -> Vec<u8> {
    let mut out = RESET.to_vec();
    if let Some(s) = size {
        out.extend_from_slice(format!("\x1b[8;{};{}t", s.rows, s.cols).as_bytes());
    }
    out
}

/// Streams one session's output to a client: reset + snapshot, then batched live output.
/// On lag (lost chunks) it resets and replays the snapshot.
fn spawn_forwarder(
    tl: Arc<Termlab>,
    id: SessionId,
    att: crate::pty::Attachment,
    tx: mpsc::Sender<(u8, Vec<u8>)>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut first = replay_prefix(att.size);
        first.extend_from_slice(&att.snapshot);
        if tx.send(output_frame(id, &first)).await.is_err() {
            return;
        }
        let mut rx = att.events;
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let mut done = false;
            match rx.recv().await {
                Ok(PtyEvent::Output(b)) => pending.extend_from_slice(&b),
                Ok(PtyEvent::Exited(_)) | Err(broadcast::error::RecvError::Closed) => done = true,
                Err(broadcast::error::RecvError::Lagged(_)) => match tl.attach(id) {
                    Ok(fresh) => {
                        pending.clear();
                        pending.extend_from_slice(&replay_prefix(fresh.size));
                        pending.extend_from_slice(&fresh.snapshot);
                        rx = fresh.events;
                    }
                    Err(_) => done = true,
                },
            }
            if !done {
                let deadline = Instant::now() + BATCH_WINDOW;
                while pending.len() < MAX_BATCH {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => break,
                        ev = rx.recv() => match ev {
                            Ok(PtyEvent::Output(b)) => pending.extend_from_slice(&b),
                            Ok(PtyEvent::Exited(_)) | Err(broadcast::error::RecvError::Closed) => { done = true; break }
                            Err(broadcast::error::RecvError::Lagged(_)) => break,
                        }
                    }
                }
            }
            if !pending.is_empty()
                && tx
                    .send(output_frame(id, &std::mem::take(&mut pending)))
                    .await
                    .is_err()
            {
                return;
            }
            if done {
                return;
            }
        }
    })
}

// ---- client -----------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ClientEvent {
    Core(CoreEvent),
    Output {
        id: SessionId,
        data: Bytes,
    },
    /// The daemon dropped events for this client; reload state.
    Resync,
    /// The connection closed.
    Disconnected,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// The app's connection to `termlabd`.
pub struct Client {
    tx: mpsc::UnboundedSender<(u8, Vec<u8>)>,
    pending: Pending,
    next_id: AtomicU64,
    events: broadcast::Sender<ClientEvent>,
}

impl Client {
    pub async fn connect(path: &Path) -> std::io::Result<Arc<Self>> {
        let stream = UnixStream::connect(path).await?;
        let (mut reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<(u8, Vec<u8>)>();
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(8192);

        tokio::spawn(async move {
            while let Some((kind, payload)) = rx.recv().await {
                if write_frame(&mut writer, kind, &payload).await.is_err() {
                    break;
                }
            }
        });

        {
            let pending = pending.clone();
            let events = events.clone();
            tokio::spawn(async move {
                while let Ok((kind, payload)) = read_frame(&mut reader).await {
                    match kind {
                        KIND_OUTPUT if payload.len() >= 16 => {
                            let mut idb = [0u8; 16];
                            idb.copy_from_slice(&payload[..16]);
                            let _ = events.send(ClientEvent::Output {
                                id: SessionId(Uuid::from_bytes(idb)),
                                data: Bytes::copy_from_slice(&payload[16..]),
                            });
                        }
                        KIND_JSON => match serde_json::from_slice::<ServerMsg>(&payload) {
                            Ok(ServerMsg::Reply { id, ok, err }) => {
                                let waiter = pending.lock().expect("pending lock").remove(&id);
                                if let Some(w) = waiter {
                                    let _ = w.send(match err {
                                        Some(e) => Err(e),
                                        None => Ok(ok.unwrap_or(Value::Null)),
                                    });
                                }
                            }
                            Ok(ServerMsg::Event { event }) => {
                                let _ = events.send(ClientEvent::Core(event));
                            }
                            Ok(ServerMsg::Resync) => {
                                let _ = events.send(ClientEvent::Resync);
                            }
                            Err(e) => tracing::warn!(error = %e, "bad message from termlabd"),
                        },
                        _ => {}
                    }
                }
                // Fail outstanding calls and tell listeners.
                for (_, w) in pending.lock().expect("pending lock").drain() {
                    let _ = w.send(Err("lost connection to termlabd".into()));
                }
                let _ = events.send(ClientEvent::Disconnected);
            });
        }

        Ok(Arc::new(Self {
            tx,
            pending,
            next_id: AtomicU64::new(1),
            events,
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ClientEvent> {
        self.events.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    pub async fn call(&self, req: Request) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (w, r) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, w);
        if self.tx.send(json_frame(&ClientMsg { id, req })).is_err() {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err("lost connection to termlabd".into());
        }
        match tokio::time::timeout(CALL_TIMEOUT, r).await {
            Ok(reply) => reply.unwrap_or_else(|_| Err("lost connection to termlabd".into())),
            Err(_) => {
                self.pending.lock().expect("pending lock").remove(&id);
                Err("Termlab's background service didn't answer in time".into())
            }
        }
    }

    pub async fn call_as<T: DeserializeOwned>(&self, req: Request) -> Result<T, String> {
        let v = self.call(req).await?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
}
