//! Wayland: programs may not read the screen or send input themselves, so this goes through
//! the desktop's portal (`org.freedesktop.portal.RemoteDesktop` with a ScreenCast stream),
//! the way GNOME's and KDE's own remote desktop work.
//!
//! - The first time, the desktop asks the person at the computer which screen to share and
//!   whether to allow control. The portal hands back a restore token, kept in the data dir
//!   (`screen-portal.json`); later sessions use it and start without asking. Termlab's
//!   settings start that first approval on purpose (`termlab-screen authorize`).
//! - The picture comes over PipeWire. Rather than linking PipeWire, the helper runs
//!   GStreamer's `gst-launch-1.0` with `pipewiresrc` (packages: Debian/Ubuntu
//!   `gstreamer1.0-tools gstreamer1.0-pipewire`, Fedora `gstreamer1-plugins-base-tools
//!   pipewire-gstreamer`) and reads raw frames from it.
//! - Input goes back through the portal (evdev key codes, as on X11 minus 8).
//!
//! One portal session per helper process, shared by the stream and the input.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use serde::{Deserialize, Serialize};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use super::super::Bounds;
use super::super::pc_keys::{keysym, pc_key};
use super::super::soft::{Grab, Image, Opened};
use crate::proto::{DisplayInfo, InputEvent};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE: &str = "org.freedesktop.portal.RemoteDesktop";
const CAST: &str = "org.freedesktop.portal.ScreenCast";
const TOKEN_FILE: &str = "screen-portal.json";
/// How long the person at the computer has to answer the desktop's dialog.
const APPROVAL: Duration = Duration::from_secs(90);

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_data_dir(dir: &Path) {
    let _ = DATA_DIR.set(dir.to_path_buf());
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    restore_token: Option<String>,
}

fn token_path(dir: &Path) -> PathBuf {
    dir.join(TOKEN_FILE)
}

fn load_token(dir: Option<&Path>) -> Option<String> {
    let dir = dir?;
    std::fs::read(token_path(dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
        .and_then(|s| s.restore_token)
        .filter(|t| !t.is_empty())
}

fn save_token(dir: Option<&Path>, token: Option<String>) {
    let (Some(dir), Some(token)) = (dir, token) else {
        return;
    };
    let body = serde_json::to_vec(&Saved {
        restore_token: Some(token),
    })
    .unwrap_or_default();
    let _ = std::fs::write(token_path(dir), body);
}

/// Approved before (so a session can start with no one at the computer).
pub fn approved() -> bool {
    load_token(DATA_DIR.get().map(PathBuf::as_path)).is_some()
}

// ---- the portal session ----------------------------------------------------------------

pub struct Portal {
    conn: Connection,
    session: String,
    node: u32,
    /// The stream's size in the portal's (logical) coordinates, which pointer motion uses.
    size: (i32, i32),
    fd: Mutex<Option<OwnedFd>>,
}

static TOKENS: AtomicU32 = AtomicU32::new(0);

fn token() -> String {
    format!(
        "termlab{}_{}",
        std::process::id(),
        TOKENS.fetch_add(1, Ordering::Relaxed)
    )
}

type Results = HashMap<String, OwnedValue>;

fn opts<'a>(pairs: Vec<(&'a str, Value<'a>)>) -> HashMap<&'a str, Value<'a>> {
    pairs.into_iter().collect()
}

impl Portal {
    /// Calls a portal method that answers through a Request object's `Response` signal.
    fn request<B>(
        conn: &Connection,
        iface: &str,
        method: &str,
        handle: &str,
        body: &B,
    ) -> anyhow::Result<Results>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let sender = conn
            .unique_name()
            .context("no D-Bus name")?
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("{PATH}/request/{sender}/{handle}");
        let request = Proxy::new(conn, DEST, path.as_str(), "org.freedesktop.portal.Request")?;
        let mut responses = request.receive_signal("Response")?;
        conn.call_method(Some(DEST), PATH, Some(iface), method, body)
            .with_context(|| format!("{iface}.{method}"))?;
        let msg = responses
            .next()
            .ok_or_else(|| anyhow!("{method}: the portal didn't answer"))?;
        let (code, results): (u32, Results) = msg.body().deserialize()?;
        match code {
            0 => Ok(results),
            1 => bail!("Screen sharing wasn't allowed on the computer."),
            _ => bail!("The desktop couldn't start screen sharing ({method})."),
        }
    }

    fn open(data_dir: Option<&Path>) -> anyhow::Result<Portal> {
        let conn = Connection::session().context("Can't reach the desktop session (D-Bus)")?;

        let h = token();
        let created = Self::request(
            &conn,
            REMOTE,
            "CreateSession",
            &h,
            &(opts(vec![
                ("handle_token", Value::from(h.as_str())),
                ("session_handle_token", Value::from(token())),
            ]),),
        )?;
        let session: String = created
            .get("session_handle")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .context("The desktop didn't open a remote desktop session.")?;
        let sp = ObjectPath::try_from(session.as_str())?;

        let h = token();
        let mut devices = vec![
            ("handle_token", Value::from(h.as_str())),
            // Keyboard and pointer.
            ("types", Value::from(3u32)),
            // Remember the approval until it's revoked.
            ("persist_mode", Value::from(2u32)),
        ];
        let saved = load_token(data_dir);
        if let Some(t) = &saved {
            devices.push(("restore_token", Value::from(t.as_str())));
        }
        Self::request(&conn, REMOTE, "SelectDevices", &h, &(&sp, opts(devices)))?;

        let h = token();
        let sources = vec![
            ("handle_token", Value::from(h.as_str())),
            // A monitor; the pointer drawn into the picture.
            ("types", Value::from(1u32)),
            ("multiple", Value::from(false)),
            ("cursor_mode", Value::from(2u32)),
        ];
        if Self::request(&conn, CAST, "SelectSources", &h, &(&sp, opts(sources))).is_err() {
            let h = token();
            let sources = vec![
                ("handle_token", Value::from(h.as_str())),
                ("types", Value::from(1u32)),
                ("multiple", Value::from(false)),
            ];
            Self::request(&conn, CAST, "SelectSources", &h, &(&sp, opts(sources)))?;
        }

        let h = token();
        let started = Self::request(
            &conn,
            REMOTE,
            "Start",
            &h,
            &(
                &sp,
                "",
                opts(vec![("handle_token", Value::from(h.as_str()))]),
            ),
        )?;
        let new_token = started
            .get("restore_token")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok());
        save_token(data_dir, new_token);
        let streams: Vec<(u32, HashMap<String, OwnedValue>)> = started
            .get("streams")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| Vec::try_from(v).ok())
            .context("The desktop didn't share a screen.")?;
        let (node, props) = streams
            .into_iter()
            .next()
            .context("The desktop didn't share a screen.")?;
        let size = props
            .get("size")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| <(i32, i32)>::try_from(Value::from(v)).ok())
            .unwrap_or((1920, 1080));

        let reply = conn.call_method(
            Some(DEST),
            PATH,
            Some(CAST),
            "OpenPipeWireRemote",
            &(&sp, HashMap::<&str, Value>::new()),
        )?;
        let fd: zbus::zvariant::OwnedFd = reply.body().deserialize()?;
        Ok(Portal {
            conn,
            session,
            node,
            size,
            fd: Mutex::new(Some(fd.into())),
        })
    }

    /// The session's portal, opened on first use (with the desktop's dialog if it was never
    /// approved, waiting up to [`APPROVAL`]).
    pub fn get() -> anyhow::Result<Arc<Portal>> {
        static PORTAL: Mutex<Option<Arc<Portal>>> = Mutex::new(None);
        let mut slot = PORTAL.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = slot.as_ref() {
            return Ok(p.clone());
        }
        let p = Arc::new(open_with_timeout(DATA_DIR.get().cloned())?);
        *slot = Some(p.clone());
        Ok(p)
    }

    fn notify<B>(&self, method: &str, body: &B)
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        if let Err(e) = self
            .conn
            .call_method(Some(DEST), PATH, Some(REMOTE), method, body)
        {
            tracing::debug!(error = %e, method, "portal input refused");
        }
    }

    fn session_path(&self) -> ObjectPath<'_> {
        ObjectPath::try_from(self.session.as_str()).expect("checked when opened")
    }
}

fn open_with_timeout(dir: Option<PathBuf>) -> anyhow::Result<Portal> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("portal".into())
        .spawn(move || {
            let _ = tx.send(Portal::open(dir.as_deref()));
        })?;
    match rx.recv_timeout(APPROVAL) {
        Ok(r) => r,
        Err(_) => bail!(
            "Nobody approved screen sharing on the computer. Approve it there once (Termlab › Settings › Remote access › Allow), then try again."
        ),
    }
}

/// `termlab-screen authorize <data-dir>`: asks now (someone is at the computer) and keeps the
/// approval for later sessions.
pub fn authorize(data_dir: &Path) -> anyhow::Result<()> {
    let p = open_with_timeout(Some(data_dir.to_path_buf()))?;
    drop(p);
    if load_token(Some(data_dir)).is_none() {
        bail!("The desktop didn't offer to remember the approval, so it will ask again each time.");
    }
    Ok(())
}

pub fn displays() -> anyhow::Result<Vec<DisplayInfo>> {
    let p = Portal::get()?;
    Ok(vec![DisplayInfo {
        id: 1,
        name: "Shared screen".into(),
        width: p.size.0.max(1) as u32,
        height: p.size.1.max(1) as u32,
        scale: 1.0,
        primary: true,
    }])
}

// ---- the picture: gst-launch-1.0 → raw BGRx frames ---------------------------------------

struct Latest {
    frame: Option<Vec<u8>>,
    ended: Option<String>,
}

struct GstGrab {
    child: Child,
    latest: Arc<Mutex<Latest>>,
    w: u32,
    h: u32,
}

/// `width=(int)1920, height=(int)1080` out of a caps line.
fn caps_size(line: &str) -> Option<(u32, u32)> {
    let num = |key: &str| -> Option<u32> {
        let at = line.find(key)? + key.len();
        let digits: String = line[at..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    };
    Some((num("width=(int)")?, num("height=(int)")?))
}

pub fn open(_display: u32) -> anyhow::Result<Opened> {
    let p = Portal::get()?;
    let remote =
        p.fd.lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .context("The screen is already being captured.")?;
    let mut fds = [0i32; 2];
    // SAFETY: a pipe for the frames; both ends are owned right away.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        bail!("Couldn't set up the capture.");
    }
    // SAFETY: fresh descriptors from pipe2, owned once each.
    let (read_end, write_end) =
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let pipeline = format!(
        "pipewiresrc fd=3 path={} always-copy=true ! videoconvert ! video/x-raw,format=BGRx ! fdsink fd=4 sync=false",
        p.node
    );
    let mut cmd = Command::new("gst-launch-1.0");
    cmd.arg("-v")
        .args(pipeline.split(' '))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let (pw, wr) = (remote.as_raw_fd(), write_end.as_raw_fd());
    // SAFETY: only async-signal-safe dup2 calls between fork and exec; dup2 clears
    // close-on-exec on the new descriptors 3 and 4.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            if libc::dup2(pw, 3) < 0 || libc::dup2(wr, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "Screen sharing on Wayland needs GStreamer's tools and PipeWire plugin on the computer (Debian/Ubuntu: gstreamer1.0-tools gstreamer1.0-pipewire)."
            )
        } else {
            anyhow!("Couldn't start the screen capture: {e}")
        }
    })?;
    drop(write_end);
    drop(remote);

    // The negotiated size comes in the verbose output, before the first frame.
    let stdout = child.stdout.take().context("capture output")?;
    let (size_tx, size_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("gst-log".into())
        .spawn(move || {
            let mut sent = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if !sent
                    && line.contains("GstFdSink")
                    && line.contains("caps = video/x-raw")
                    && let Some(size) = caps_size(&line)
                {
                    let _ = size_tx.send(size);
                    sent = true;
                }
            }
        })?;
    let (w, h) = match size_rx.recv_timeout(Duration::from_secs(15)) {
        Ok(s) => s,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "The screen capture didn't start. Is GStreamer's PipeWire plugin installed (Debian/Ubuntu: gstreamer1.0-pipewire)?"
            );
        }
    };

    let latest = Arc::new(Mutex::new(Latest {
        frame: None,
        ended: None,
    }));
    {
        let latest = latest.clone();
        let mut reader = std::fs::File::from(read_end);
        let len = w as usize * h as usize * 4;
        std::thread::Builder::new()
            .name("gst-frames".into())
            .spawn(move || {
                loop {
                    let mut buf = vec![0u8; len];
                    if let Err(e) = reader.read_exact(&mut buf) {
                        latest.lock().unwrap_or_else(|e| e.into_inner()).ended =
                            Some(format!("GStreamer's output ended ({e})"));
                        return;
                    }
                    latest.lock().unwrap_or_else(|e| e.into_inner()).frame = Some(buf);
                }
            })?;
    }
    Ok(Opened {
        grab: Box::new(GstGrab {
            child,
            latest,
            w,
            h,
        }),
        // The portal draws the pointer into the picture (it can't be had apart through
        // `gst-launch`).
        pointer: None,
        display: 1,
        bounds: Bounds {
            x: 0.0,
            y: 0.0,
            w: f64::from(p.size.0.max(1)),
            h: f64::from(p.size.1.max(1)),
        },
        width: w,
        height: h,
    })
}

impl Grab for GstGrab {
    fn grab(&mut self) -> anyhow::Result<Option<Image>> {
        let mut l = self.latest.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = &l.ended {
            bail!("{e}");
        }
        Ok(l.frame.take().map(|data| Image {
            data,
            width: self.w,
            height: self.h,
            stride: self.w as usize * 4,
        }))
    }
}

impl Drop for GstGrab {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---- input -------------------------------------------------------------------------------

/// evdev buttons for the browser's left, right and middle.
const BUTTONS: [i32; 3] = [0x110, 0x111, 0x112];

pub struct Input {
    portal: Option<Arc<Portal>>,
    keys: HashSet<i32>,
    buttons: [bool; 3],
}

impl Input {
    pub fn new() -> Self {
        Input {
            portal: None,
            keys: HashSet::new(),
            buttons: [false; 3],
        }
    }

    fn portal(&mut self) -> Option<Arc<Portal>> {
        if self.portal.is_none() {
            self.portal = Portal::get().ok();
        }
        self.portal.clone()
    }

    pub fn apply(&mut self, ev: InputEvent) {
        let Some(p) = self.portal() else { return };
        let s = p.session_path();
        let none = HashMap::<&str, Value>::new();
        match ev {
            InputEvent::Move { x, y } => {
                p.notify("NotifyPointerMotionAbsolute", &(&s, &none, p.node, x, y));
            }
            InputEvent::Button {
                x, y, button, down, ..
            } => {
                let Some(&b) = BUTTONS.get(button) else {
                    return;
                };
                p.notify("NotifyPointerMotionAbsolute", &(&s, &none, p.node, x, y));
                p.notify("NotifyPointerButton", &(&s, &none, b, u32::from(down)));
                self.buttons[button] = down;
            }
            InputEvent::Wheel { dx, dy } => {
                let finish = opts(vec![("finish", Value::from(false))]);
                p.notify("NotifyPointerAxis", &(&s, &finish, dx, dy));
            }
            InputEvent::Key { code, down, .. } => {
                let Some(k) = pc_key(&code) else { return };
                let key = i32::from(k.evdev);
                if down {
                    self.keys.insert(key);
                } else {
                    self.keys.remove(&key);
                }
                p.notify("NotifyKeyboardKeycode", &(&s, &none, key, u32::from(down)));
            }
            InputEvent::Text { text } => {
                for c in text.chars() {
                    let sym = keysym(c) as i32;
                    p.notify("NotifyKeyboardKeysym", &(&s, &none, sym, 1u32));
                    p.notify("NotifyKeyboardKeysym", &(&s, &none, sym, 0u32));
                }
            }
            InputEvent::ReleaseAll => self.release_all(),
        }
    }

    pub fn release_all(&mut self) {
        let Some(p) = self.portal.clone() else { return };
        let s = p.session_path();
        let none = HashMap::<&str, Value>::new();
        for key in std::mem::take(&mut self.keys) {
            p.notify("NotifyKeyboardKeycode", &(&s, &none, key, 0u32));
        }
        for (i, down) in self.buttons.iter_mut().enumerate() {
            if *down {
                p.notify("NotifyPointerButton", &(&s, &none, BUTTONS[i], 0u32));
                *down = false;
            }
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_from_caps() {
        let line = "/GstPipeline:pipeline0/GstFdSink:fdsink0.GstPad:sink: caps = video/x-raw, format=(string)BGRx, width=(int)3840, height=(int)2160, framerate=(fraction)0/1";
        assert_eq!(caps_size(line), Some((3840, 2160)));
        assert_eq!(caps_size("caps = video/x-raw"), None);
    }
}
