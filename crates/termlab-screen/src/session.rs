//! One screen session: the websocket to the server, the capture stream, and the browser's
//! input.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::unbounded_channel;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::platform::{self, Input, Stream};
use crate::proto::{
    FromBrowser, InputEvent, PointerAction, Quality, annex_b, button_index, video_frame,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Sharing switched on and off in Termlab's settings (written by the app).
const SETTINGS_FILE: &str = "screen.json";
/// While a session runs: who is watching, for the app's banner and its Stop button.
const ACTIVE_FILE: &str = "screen-session.json";
const TICK: Duration = Duration::from_secs(1);
const PING: Duration = Duration::from_secs(20);
/// No acknowledgement for this long while frames are out: open the window again.
const ACK_TIMEOUT: Duration = Duration::from_secs(5);
/// How often to tell macOS the user is active (no screensaver, no lock, while controlled).
const AWAKE_EVERY: Duration = Duration::from_secs(30);
const MAX_TEXT: usize = 10_000;
const MAX_CLIPBOARD: usize = 1 << 20;
/// How long the person at the computer has to allow a session.
const CONFIRM_SECS: u32 = 60;

/// What termlabd passes on stdin.
#[derive(Deserialize)]
pub struct Launch {
    pub server_url: String,
    pub token: String,
    pub sid: String,
    pub data_dir: PathBuf,
    /// Who asked (address and browser), for the local prompt.
    #[serde(default)]
    pub viewer: String,
}

#[derive(Deserialize, Default)]
struct Settings {
    #[serde(default)]
    enabled: bool,
    /// Ask on this computer before each session.
    #[serde(default)]
    confirm: bool,
}

fn settings(data_dir: &Path) -> Settings {
    std::fs::read(data_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn init_logging(data_dir: &Path) {
    let Ok(log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("termlab-screen.log"))
    else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(log))
        .try_init();
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn ws_url(server_url: &str, sid: &str) -> anyhow::Result<String> {
    let base = server_url.trim_end_matches('/');
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        bail!("{server_url} isn't a valid server address.");
    };
    let sid: String = sid
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    Ok(format!("{base}/api/machine/screen/ws?sid={sid}"))
}

async fn connect(l: &Launch) -> anyhow::Result<Ws> {
    let url = ws_url(&l.server_url, &l.sid)?;
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {}", l.token).parse()?);
    request
        .headers_mut()
        .insert("user-agent", format!("Termlab/{VERSION}").parse()?);
    let connector = if url.starts_with("wss://") {
        use rustls_platform_verifier::ConfigVerifierExt;
        let tls = rustls::ClientConfig::with_platform_verifier()?;
        Some(tokio_tungstenite::Connector::Rustls(Arc::new(tls)))
    } else {
        None
    };
    let (ws, _) = tokio_tungstenite::connect_async_tls_with_config(request, None, true, connector)
        .await
        .context("Couldn't reach the server")?;
    Ok(ws)
}

async fn send_json(ws: &mut Ws, v: Value) -> anyhow::Result<()> {
    ws.send(Message::Text(v.to_string().into())).await?;
    Ok(())
}

/// Asks the person at this computer. True when they allow it.
async fn confirm(viewer: &str) -> bool {
    let who = if viewer.is_empty() {
        "Someone signed in to your Termlab server"
    } else {
        viewer
    };
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display dialog \"Allow remote control of this computer?\\n\\n{}\\nwants to see your screen and use your mouse and keyboard, through your Termlab server.\" \
         with title \"Termlab\" buttons {{\"Deny\", \"Allow\"}} default button \"Deny\" cancel button \"Deny\" with icon caution giving up after {CONFIRM_SECS}",
        esc(who)
    );
    let out = tokio::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(u64::from(CONFIRM_SECS) + 5), out).await {
        Ok(Ok(o)) => {
            let text = String::from_utf8_lossy(&o.stdout);
            o.status.success()
                && text.contains("button returned:Allow")
                && !text.contains("gave up:true")
        }
        _ => false,
    }
}

/// Keeps the display on while the session runs (`caffeinate` ends with this process).
fn keep_awake() -> Option<std::process::Child> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    std::process::Command::new("/usr/bin/caffeinate")
        .args(["-d", "-i", "-w", &std::process::id().to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()
}

/// Counts as user activity: no screensaver and no screen lock while someone is connected.
fn declare_activity() {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("/usr/bin/caffeinate")
            .args(["-u", "-t", "1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

async fn clipboard_get() -> Option<String> {
    let out = tokio::process::Command::new("/usr/bin/pbpaste")
        .output()
        .await
        .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.len() > MAX_CLIPBOARD {
        let mut end = MAX_CLIPBOARD;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Some(text)
}

async fn clipboard_set(text: &str) -> bool {
    let Ok(mut child) = tokio::process::Command::new("/usr/bin/pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes()).await;
    }
    child.wait().await.map(|s| s.success()).unwrap_or(false)
}

/// Removes the "session running" file when the session ends, however it ends.
struct ActiveFile(PathBuf);

impl ActiveFile {
    fn write(data_dir: &Path, viewer: &str) -> Self {
        let path = data_dir.join(ACTIVE_FILE);
        let body = json!({
            "pid": std::process::id(),
            "started_at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            "viewer": viewer,
        });
        let _ = std::fs::write(&path, body.to_string());
        ActiveFile(path)
    }
}

impl Drop for ActiveFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn start_stream(
    display: u32,
    quality: Quality,
    tx: &tokio::sync::mpsc::UnboundedSender<crate::proto::Encoded>,
) -> anyhow::Result<Stream> {
    tokio::task::block_in_place(|| Stream::start(display, quality, tx.clone()))
}

pub async fn serve() -> anyhow::Result<()> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading the launch request")?;
    let launch: Launch = serde_json::from_str(line.trim()).context("parsing the launch request")?;
    init_logging(&launch.data_dir);
    tracing::info!(sid = %launch.sid, "screen session starting");
    let result = run(&launch).await;
    match &result {
        Ok(()) => tracing::info!("screen session ended"),
        Err(e) => tracing::warn!(error = %e, "screen session failed"),
    }
    result
}

async fn fail(ws: &mut Ws, message: &str) -> anyhow::Result<()> {
    send_json(ws, json!({ "type": "error", "message": message })).await?;
    let _ = ws.close(None).await;
    Ok(())
}

async fn run(launch: &Launch) -> anyhow::Result<()> {
    let mut ws = connect(launch).await?;
    if !settings(&launch.data_dir).enabled {
        return fail(
            &mut ws,
            "Screen sharing is switched off on this computer (Termlab › Settings › Remote access).",
        )
        .await;
    }
    let perms = platform::permissions();
    if !perms.screen {
        return fail(
            &mut ws,
            "Termlab isn't allowed to record this computer's screen. Allow it on the computer: Termlab › Settings › Remote access.",
        )
        .await;
    }
    if settings(&launch.data_dir).confirm {
        send_json(&mut ws, json!({ "type": "status", "state": "confirming" })).await?;
        if !confirm(&launch.viewer).await {
            return fail(&mut ws, "The person at the computer didn't allow it.").await;
        }
    }

    let displays = tokio::task::block_in_place(platform::displays)?;
    let mut display = displays
        .first()
        .map(|d| d.id)
        .context("There is no display to share.")?;
    let mut quality = Quality::Balanced;
    send_json(
        &mut ws,
        json!({
            "type": "hello", "version": VERSION, "os": std::env::consts::OS,
            "displays": displays, "display": display, "quality": quality, "input": perms.input,
        }),
    )
    .await?;

    let _active = ActiveFile::write(&launch.data_dir, &launch.viewer);
    let mut awake = keep_awake();
    let (tx, mut rx) = unbounded_channel();
    let mut stream = start_stream(display, quality, &tx)?;
    let mut input = Input::new();
    let mut seq: u32 = 0;
    let mut last_ack = Instant::now();
    let mut last_awake = Instant::now();
    let mut tick = tokio::time::interval(TICK);
    let mut ping = tokio::time::interval(PING);
    ping.reset();
    let mut refine = tokio::time::interval(platform::REFINE_TICK);
    refine.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    declare_activity();

    let result: anyhow::Result<()> = loop {
        tokio::select! {
            Some(frame) = rx.recv() => {
                if let Some(cfg) = &frame.config {
                    send_json(&mut ws, json!({
                        "type": "config", "codec": cfg.codec, "description": B64.encode(&cfg.description),
                        "width": cfg.width, "height": cfg.height, "display": stream.display(),
                    })).await?;
                }
                seq = seq.wrapping_add(1);
                ws.send(Message::Binary(video_frame(seq, &frame).into())).await?;
            }
            msg = ws.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Ping(p))) => { ws.send(Message::Pong(p)).await?; continue }
                    Some(Ok(Message::Close(_))) | None => break Ok(()),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => break Err(e.into()),
                };
                let Ok(m) = serde_json::from_str::<FromBrowser>(&text) else { continue };
                let b = stream.bounds();
                let at = |x: f64, y: f64| (b.x + x.clamp(0.0, 1.0) * b.w, b.y + y.clamp(0.0, 1.0) * b.h);
                match m {
                    FromBrowser::Ack { .. } => { stream.ack(); last_ack = Instant::now(); }
                    FromBrowser::Keyframe => stream.keyframe(),
                    FromBrowser::Display { id } => {
                        if id != display && displays.iter().any(|d| d.id == id) {
                            display = id;
                            drop(stream);
                            stream = start_stream(display, quality, &tx)?;
                            send_json(&mut ws, json!({ "type": "display", "display": display })).await?;
                        }
                    }
                    FromBrowser::Quality { quality: q } => {
                        if q != quality {
                            quality = q;
                            drop(stream);
                            stream = start_stream(display, quality, &tx)?;
                            send_json(&mut ws, json!({ "type": "quality", "quality": quality })).await?;
                        }
                    }
                    FromBrowser::Pointer { x, y, action, button, clicks } if perms.input => {
                        let (gx, gy) = at(x, y);
                        match (action, button_index(button)) {
                            (PointerAction::Move, _) => input.apply(InputEvent::Move { x: gx, y: gy }),
                            (PointerAction::Down, Some(b)) => input.apply(InputEvent::Button { x: gx, y: gy, button: b, down: true, clicks }),
                            (PointerAction::Up, Some(b)) => input.apply(InputEvent::Button { x: gx, y: gy, button: b, down: false, clicks }),
                            _ => {}
                        }
                        last_awake = Instant::now();
                    }
                    FromBrowser::Wheel { dx, dy } if perms.input => input.apply(InputEvent::Wheel { dx, dy }),
                    FromBrowser::Key { code, down, repeat } if perms.input => {
                        input.apply(InputEvent::Key { code, down, repeat });
                        last_awake = Instant::now();
                    }
                    FromBrowser::Text { text } if perms.input => {
                        let text: String = text.chars().take(MAX_TEXT).collect();
                        input.apply(InputEvent::Text { text });
                    }
                    FromBrowser::Release => input.apply(InputEvent::ReleaseAll),
                    FromBrowser::ClipboardSet { text } => {
                        if text.len() <= MAX_CLIPBOARD {
                            let ok = clipboard_set(&text).await;
                            send_json(&mut ws, json!({ "type": "clipboard.saved", "ok": ok })).await?;
                        }
                    }
                    FromBrowser::ClipboardGet => {
                        let text = clipboard_get().await.unwrap_or_default();
                        send_json(&mut ws, json!({ "type": "clipboard", "text": text })).await?;
                    }
                    _ => {}
                }
            }
            _ = tick.tick() => {
                if stream.inflight() > 0 && last_ack.elapsed() > ACK_TIMEOUT {
                    stream.reset_window();
                    last_ack = Instant::now();
                }
                if !settings(&launch.data_dir).enabled {
                    let _ = send_json(&mut ws, json!({ "type": "error", "message": "Screen sharing was switched off on the computer." })).await;
                    break Ok(());
                }
                if last_awake.elapsed() > AWAKE_EVERY {
                    declare_activity();
                    last_awake = Instant::now();
                }
            }
            _ = ping.tick() => { ws.send(Message::Ping(Vec::new().into())).await?; }
            _ = refine.tick() => stream.refine(),
        }
    };

    input.apply(InputEvent::ReleaseAll);
    drop(stream);
    if let Some(mut c) = awake.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = ws.close(None).await;
    result
}

/// Development check: capture the main display for `secs` seconds into an H.264 file.
pub async fn probe(secs: u64, out: PathBuf) -> anyhow::Result<()> {
    let perms = platform::permissions();
    eprintln!("permissions: screen={} input={}", perms.screen, perms.input);
    let displays = tokio::task::block_in_place(platform::displays)?;
    eprintln!("displays: {}", serde_json::to_string(&displays)?);
    let display = displays.first().map(|d| d.id).context("no display")?;
    let (tx, mut rx) = unbounded_channel();
    let started = Instant::now();
    let quality = match std::env::var("PROBE_QUALITY").as_deref() {
        Ok("low") => Quality::Low,
        Ok("high") => Quality::High,
        _ => Quality::Balanced,
    };
    let stream = start_stream(display, quality, &tx)?;
    eprintln!("capture started in {:?}", started.elapsed());
    let mut file = std::fs::File::create(&out)?;
    let mut avcc = Vec::new();
    let (mut frames, mut keys, mut bytes) = (0u32, 0u32, 0usize);
    let mut asked_key = false;
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        if !asked_key && Instant::now() + Duration::from_secs(secs) / 2 > end {
            stream.keyframe();
            asked_key = true;
        }
        match tokio::time::timeout(left.min(platform::REFINE_TICK), rx.recv()).await {
            Ok(Some(f)) => {
                if let Some(c) = &f.config {
                    eprintln!(
                        "config: {} {}x{} ({} byte avcC)",
                        c.codec,
                        c.width,
                        c.height,
                        c.description.len()
                    );
                    avcc = c.description.clone();
                }
                frames += 1;
                keys += u32::from(f.key);
                bytes += f.data.len();
                eprintln!(
                    "  {:>6} ms  {:>8} bytes{}",
                    started.elapsed().as_millis(),
                    f.data.len(),
                    if f.key { "  key" } else { "" }
                );
                file.write_all(&annex_b(&avcc, &f.data, f.key))?;
                stream.ack();
            }
            Ok(None) => break,
            Err(_) => stream.refine(),
        }
    }
    drop(stream);
    eprintln!(
        "{frames} frames ({keys} key) in {secs}s, {} KB, {:.0} kbit/s → {}",
        bytes / 1024,
        bytes as f64 * 8.0 / 1000.0 / secs as f64,
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_follows_the_server_scheme() {
        assert_eq!(
            ws_url("https://r.example.com/", "ab-12").unwrap(),
            "wss://r.example.com/api/machine/screen/ws?sid=ab-12"
        );
        assert_eq!(
            ws_url("http://127.0.0.1:8484", "x\"y").unwrap(),
            "ws://127.0.0.1:8484/api/machine/screen/ws?sid=xy"
        );
        assert!(ws_url("ftp://x", "a").is_err());
    }
}
