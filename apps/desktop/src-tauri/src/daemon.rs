//! The app's link to `termlabd`: start it if needed, keep the connection, route output to
//! the terminal views, and reconnect (re-attaching every open view) if it drops.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter};
use termlab_core::DataDirs;
use termlab_core::ipc::{Client, ClientEvent, DaemonInfo, PROTOCOL_VERSION, Request};
use termlab_core::model::SessionId;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub connected: bool,
    pub message: Option<String>,
}

pub struct Daemon {
    dirs: DataDirs,
    client: RwLock<Option<Arc<Client>>>,
    /// Terminal views currently showing each session.
    views: Mutex<HashMap<SessionId, Channel<InvokeResponseBody>>>,
    status: Mutex<DaemonStatus>,
    /// Set (to its session count) when the daemon is an older protocol kept for its sessions.
    outdated: Mutex<Option<usize>>,
}

impl Daemon {
    pub fn new(dirs: DataDirs) -> Arc<Self> {
        Arc::new(Self {
            dirs,
            client: RwLock::new(None),
            views: Mutex::new(HashMap::new()),
            status: Mutex::new(DaemonStatus {
                connected: false,
                message: Some("Starting the Termlab background service…".into()),
            }),
            outdated: Mutex::new(None),
        })
    }

    pub fn status(&self) -> DaemonStatus {
        self.status.lock().expect("status lock").clone()
    }

    fn set_status(&self, app: &AppHandle, connected: bool, message: Option<String>) {
        let s = DaemonStatus { connected, message };
        *self.status.lock().expect("status lock") = s.clone();
        let _ = app.emit("daemon-status", s);
    }

    fn client(&self) -> Result<Arc<Client>, String> {
        self.client
            .read()
            .expect("client lock")
            .clone()
            .ok_or_else(|| "Not connected to the Termlab background service yet".to_string())
    }

    pub async fn call(&self, req: Request) -> Result<serde_json::Value, String> {
        self.client()?.call(req).await
    }

    pub async fn attach(
        &self,
        id: SessionId,
        channel: Channel<InvokeResponseBody>,
    ) -> Result<serde_json::Value, String> {
        self.views.lock().expect("views lock").insert(id, channel);
        self.call(Request::Attach { id }).await
    }

    pub async fn detach(&self, id: SessionId) -> Result<(), String> {
        self.views.lock().expect("views lock").remove(&id);
        self.call(Request::Detach { id }).await.map(|_| ())
    }

    /// Connects (starting the daemon if needed) and keeps reconnecting while the app runs.
    pub async fn run(self: Arc<Self>, app: AppHandle) {
        loop {
            match self.connect().await {
                Ok(client) => {
                    let mut events = client.subscribe();
                    *self.client.write().expect("client lock") = Some(client.clone());
                    let outdated = *self.outdated.lock().expect("outdated lock");
                    self.set_status(
                        &app,
                        true,
                        outdated.map(|n| {
                            format!(
                                "Termlab's background service is from an older version and is running {n} session{}. \
                                 New features need a restart: quit Termlab once those sessions are done, and it restarts on next launch.",
                                if n == 1 { "" } else { "s" }
                            )
                        }),
                    );
                    // Re-attach views that were open before a reconnect; each gets a reset
                    // plus its full scrollback.
                    let ids: Vec<SessionId> = self
                        .views
                        .lock()
                        .expect("views lock")
                        .keys()
                        .copied()
                        .collect();
                    for id in ids {
                        let _ = client.call(Request::Attach { id }).await;
                    }
                    let _ = app.emit("core-resync", ());
                    self.pump(&app, &mut events).await;
                    *self.client.write().expect("client lock") = None;
                    self.set_status(
                        &app,
                        false,
                        Some("Reconnecting to the Termlab background service…".into()),
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "could not reach termlabd");
                    self.set_status(&app, false, Some(e));
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    /// Forwards daemon events until the connection drops.
    async fn pump(
        &self,
        app: &AppHandle,
        events: &mut tokio::sync::broadcast::Receiver<ClientEvent>,
    ) {
        loop {
            match events.recv().await {
                Ok(ClientEvent::Output { id, data }) => {
                    let channel = self.views.lock().expect("views lock").get(&id).cloned();
                    if let Some(ch) = channel {
                        let _ = ch.send(InvokeResponseBody::Raw(data.to_vec()));
                    }
                }
                Ok(ClientEvent::Core(e)) => {
                    let _ = app.emit("core-event", e);
                }
                Ok(ClientEvent::Resync) => {
                    let _ = app.emit("core-resync", ());
                }
                Ok(ClientEvent::Disconnected)
                | Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // Output chunks were dropped: replay every open view, reload state.
                    tracing::warn!(n, "app fell behind the daemon; re-attaching");
                    let client = self.client.read().expect("client lock").clone();
                    if let Some(client) = client {
                        let ids: Vec<SessionId> = self
                            .views
                            .lock()
                            .expect("views lock")
                            .keys()
                            .copied()
                            .collect();
                        for id in ids {
                            let _ = client.call(Request::Attach { id }).await;
                        }
                    }
                    let _ = app.emit("core-resync", ());
                }
            }
        }
    }

    async fn connect(&self) -> Result<Arc<Client>, String> {
        let socket = self.dirs.daemon_socket();
        if let Ok(client) = Client::connect(&socket).await {
            match self.check_version(&client).await {
                Ok(()) => return Ok(client),
                Err(reason) => {
                    tracing::info!(%reason, "restarting termlabd");
                    let _ = client.call(Request::Shutdown { force: false }).await;
                    wait_gone(&socket).await;
                }
            }
        }
        let bin = sibling_binary("termlabd").ok_or("termlabd wasn't found next to the app")?;
        spawn_detached(&bin, &self.dirs.root)
            .map_err(|e| format!("couldn't start termlabd: {e}"))?;
        let start = tokio::time::Instant::now();
        loop {
            if let Ok(client) = Client::connect(&socket).await {
                return Ok(client);
            }
            if start.elapsed() > CONNECT_TIMEOUT {
                return Err("The Termlab background service didn't start. See termlabd.log in the app data folder.".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// A daemon from an older build is replaced when it has no running sessions.
    async fn check_version(&self, client: &Client) -> Result<(), String> {
        let info: DaemonInfo = client
            .call_as(Request::Hello {
                protocol: PROTOCOL_VERSION,
            })
            .await?;
        let stale_protocol = info.protocol != PROTOCOL_VERSION;
        let newer_binary = sibling_binary("termlabd")
            .and_then(|p| std::fs::metadata(p).ok()?.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|t| t.as_millis() as i64 > info.started_at);
        if (stale_protocol || newer_binary) && info.live_sessions == 0 {
            return Err(if stale_protocol {
                "protocol changed"
            } else {
                "newer build"
            }
            .into());
        }
        if stale_protocol {
            tracing::warn!(
                "termlabd speaks protocol {} (app {}) but has running sessions; keeping it",
                info.protocol,
                PROTOCOL_VERSION
            );
            *self.outdated.lock().expect("outdated lock") = Some(info.live_sessions);
        } else {
            *self.outdated.lock().expect("outdated lock") = None;
        }
        Ok(())
    }
}

async fn wait_gone(socket: &Path) {
    for _ in 0..50 {
        if Client::connect(socket).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `termlabd` and `termlab-hook` ship next to the app's executable (Contents/MacOS in the
/// bundle; target/<profile> in development).
pub fn sibling_binary(name: &str) -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join(name);
    p.exists().then_some(p)
}

/// Starts the daemon in its own process group, so it outlives the app.
fn spawn_detached(bin: &Path, data_dir: &Path) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = cmd.spawn()?;
    // Reap it if it exits while the app is still running.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
