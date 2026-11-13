//! A process running in a pseudo-terminal, with its output fanned out to subscribers and
//! captured in a scrollback buffer.
//!
//! Threads per session:
//! - reader: blocking reads from the pty master → scrollback + broadcast
//! - writer: drains a channel into the pty master, so a full pty input buffer (macOS's is
//!   ~1 KiB) never blocks the caller
//! - waiter: waits for the child to exit, lets the reader drain, then publishes the exit

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use bytes::Bytes;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::error::{CoreError, Result};
use crate::model::{Millis, TermSize, now_ms};
use crate::scrollback::Scrollback;

/// How many output chunks a slow subscriber may fall behind before it is told it lagged.
const BROADCAST_CAPACITY: usize = 1024;
const READ_BUF: usize = 64 * 1024;
/// After the child exits, how long to wait for the reader to drain remaining output.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(750);

/// What to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The complete environment (not additions): the child sees exactly this.
    pub env: Vec<(String, String)>,
    pub size: TermSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitInfo {
    /// Exit code. A process killed by a signal reports 1 (portable-pty's convention).
    pub code: i32,
    pub success: bool,
}

#[derive(Debug, Clone)]
pub enum PtyEvent {
    Output(Bytes),
    Exited(ExitInfo),
}

/// A consistent view for a new viewer: everything printed so far, plus a receiver for
/// everything after it (no gap, no overlap).
pub struct Attachment {
    pub snapshot: Vec<u8>,
    pub events: broadcast::Receiver<PtyEvent>,
    pub exited: Option<ExitInfo>,
    /// The terminal size the snapshot was drawn at. Replaying TUI output at any other
    /// size garbles it (cursor-addressed redraws land in the wrong places).
    pub size: Option<TermSize>,
}

struct Shared {
    scrollback: Scrollback,
    /// A headless terminal fed the same bytes, for status heuristics and previews.
    screen: vt100::Parser,
    exited: Option<ExitInfo>,
    size: TermSize,
}

/// Plain-text view of the headless terminal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenText {
    /// Visible rows, right-trimmed.
    pub rows: Vec<String>,
    /// Whether the app enabled bracketed paste mode.
    pub bracketed_paste: bool,
    /// Whether arrow keys should be sent in application mode (`ESC O A`).
    pub application_cursor: bool,
}

impl ScreenText {
    /// The last `n` non-empty rows.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let mut out: Vec<String> = self
            .rows
            .iter()
            .rev()
            .filter(|r| !r.trim().is_empty())
            .take(n)
            .cloned()
            .collect();
        out.reverse();
        out
    }
}

fn new_screen(size: TermSize) -> vt100::Parser {
    vt100::Parser::new(nz(size.rows), nz(size.cols), 0)
}

fn nz(n: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(n.max(1)).expect("non-zero")
}

pub struct PtyProcess {
    shared: Arc<Mutex<Shared>>,
    tx: broadcast::Sender<PtyEvent>,
    input: mpsc::Sender<Vec<u8>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    exit_rx: watch::Receiver<Option<ExitInfo>>,
    pid: Option<u32>,
    last_output_at: Arc<AtomicI64>,
    last_input_at: AtomicI64,
}

impl PtyProcess {
    pub fn spawn(spec: &SpawnSpec, scrollback_cap: usize) -> Result<Self> {
        Self::spawn_with_history(spec, Scrollback::new(scrollback_cap))
    }

    /// Spawns with a pre-filled scrollback (e.g. a resumed session's earlier output).
    pub fn spawn_with_history(spec: &SpawnSpec, scrollback: Scrollback) -> Result<Self> {
        if !spec.cwd.is_dir() {
            return Err(CoreError::NotADirectory(spec.cwd.clone()));
        }
        let pty = native_pty_system()
            .openpty(pty_size(spec.size))
            .map_err(|e| CoreError::Pty(e.to_string()))?;

        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        cmd.env_clear();
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let mut child = pty
            .slave
            .spawn_command(cmd)
            .map_err(|e| CoreError::Pty(format!("{}: {e}", spec.program.display())))?;
        // Drop our copy of the slave so the master sees EOF once the child exits.
        drop(pty.slave);

        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pty
            .master
            .try_clone_reader()
            .map_err(|e| CoreError::Pty(e.to_string()))?;
        let mut writer = pty
            .master
            .take_writer()
            .map_err(|e| CoreError::Pty(e.to_string()))?;

        let shared = Arc::new(Mutex::new(Shared {
            scrollback,
            screen: new_screen(spec.size),
            exited: None,
            size: spec.size,
        }));
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (exit_tx, exit_rx) = watch::channel(None);
        let last_output_at = Arc::new(AtomicI64::new(0));
        let (drained_tx, drained_rx) = mpsc::channel::<()>();

        // Reader.
        {
            let shared = shared.clone();
            let tx = tx.clone();
            let last_output_at = last_output_at.clone();
            std::thread::Builder::new()
                .name("pty-reader".into())
                .spawn(move || {
                    let mut buf = vec![0u8; READ_BUF];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                let chunk = Bytes::copy_from_slice(&buf[..n]);
                                // Push and send under one lock so `attach` sees a consistent cut.
                                let mut s = shared.lock().expect("pty shared lock");
                                s.scrollback.push(&chunk);
                                s.screen.process(&chunk);
                                let _ = tx.send(PtyEvent::Output(chunk));
                                drop(s);
                                last_output_at.store(now_ms(), Ordering::Relaxed);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            // EIO is how macOS/Linux report "slave closed".
                            Err(_) => break,
                        }
                    }
                    let _ = drained_tx.send(());
                })
                .map_err(CoreError::Io)?;
        }

        // Writer.
        let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name("pty-writer".into())
            .spawn(move || {
                for data in input_rx {
                    if writer
                        .write_all(&data)
                        .and_then(|_| writer.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(CoreError::Io)?;

        // Waiter.
        {
            let shared = shared.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("pty-waiter".into())
                .spawn(move || {
                    let info = match child.wait() {
                        Ok(status) => ExitInfo {
                            code: status.exit_code() as i32,
                            success: status.success(),
                        },
                        Err(e) => {
                            tracing::warn!(error = %e, "waiting for pty child failed");
                            ExitInfo {
                                code: -1,
                                success: false,
                            }
                        }
                    };
                    // Let the reader deliver the last output before announcing the exit. If a
                    // grandchild still holds the pty open we stop waiting after a while.
                    let _ = drained_rx.recv_timeout(DRAIN_TIMEOUT);
                    let mut s = shared.lock().expect("pty shared lock");
                    s.exited = Some(info);
                    let _ = tx.send(PtyEvent::Exited(info));
                    drop(s);
                    let _ = exit_tx.send(Some(info));
                })
                .map_err(CoreError::Io)?;
        }

        Ok(Self {
            shared,
            tx,
            input: input_tx,
            master: Mutex::new(pty.master),
            killer: Mutex::new(killer),
            exit_rx,
            pid,
            last_output_at,
            last_input_at: AtomicI64::new(0),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn attach(&self) -> Attachment {
        let s = self.shared.lock().expect("pty shared lock");
        Attachment {
            snapshot: s.scrollback.snapshot(),
            events: self.tx.subscribe(),
            exited: s.exited,
            size: Some(s.size),
        }
    }

    /// Subscribes to future events only.
    pub fn subscribe(&self) -> broadcast::Receiver<PtyEvent> {
        self.tx.subscribe()
    }

    pub fn scrollback_snapshot(&self) -> Vec<u8> {
        self.shared
            .lock()
            .expect("pty shared lock")
            .scrollback
            .snapshot()
    }

    /// The terminal's current size.
    pub fn size(&self) -> TermSize {
        self.shared.lock().expect("pty shared lock").size
    }

    pub fn screen(&self) -> ScreenText {
        let s = self.shared.lock().expect("pty shared lock");
        let screen = s.screen.screen();
        let (_, cols) = screen.size();
        ScreenText {
            rows: screen
                .rows(0, cols.get())
                .map(|r| r.trim_end().to_string())
                .collect(),
            bracketed_paste: screen.bracketed_paste(),
            application_cursor: screen.application_cursor(),
        }
    }

    pub fn flush_scrollback(&self, path: &std::path::Path) -> std::io::Result<()> {
        let data = self.scrollback_snapshot();
        crate::scrollback::write_atomically(path, &data)
    }

    /// When the process last printed anything (0 if never).
    /// When input was last written to the terminal (0 if never).
    pub fn last_input_at(&self) -> Millis {
        self.last_input_at.load(Ordering::Relaxed)
    }

    pub fn last_output_at(&self) -> Millis {
        self.last_output_at.load(Ordering::Relaxed)
    }

    pub fn exit_info(&self) -> Option<ExitInfo> {
        *self.exit_rx.borrow()
    }

    pub async fn wait(&self) -> ExitInfo {
        let mut rx = self.exit_rx.clone();
        loop {
            if let Some(info) = *rx.borrow_and_update() {
                return info;
            }
            if rx.changed().await.is_err() {
                // Sender gone without a value: treat as abnormal exit.
                return (*rx.borrow()).unwrap_or(ExitInfo {
                    code: -1,
                    success: false,
                });
            }
        }
    }

    pub fn write(&self, data: impl Into<Vec<u8>>) -> Result<()> {
        self.last_input_at.store(now_ms(), Ordering::Relaxed);
        self.input
            .send(data.into())
            .map_err(|_| CoreError::Pty("terminal input is closed".into()))
    }

    pub fn resize(&self, size: TermSize) -> Result<()> {
        if size.cols == 0 || size.rows == 0 {
            return Err(CoreError::InvalidInput(
                "terminal size must be positive".into(),
            ));
        }
        self.master
            .lock()
            .expect("pty master lock")
            .resize(pty_size(size))
            .map_err(|e| CoreError::Pty(e.to_string()))?;
        let mut shared = self.shared.lock().expect("pty shared lock");
        shared.screen.set_size(nz(size.rows), nz(size.cols));
        shared.size = size;
        Ok(())
    }

    /// Sends SIGHUP (the "terminal closed" signal) to the child.
    pub fn kill(&self) -> Result<()> {
        if self.exit_info().is_some() {
            return Ok(());
        }
        self.killer
            .lock()
            .expect("pty killer lock")
            .kill()
            .map_err(CoreError::Io)
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

fn pty_size(size: TermSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
