//! Hardware H.264 on Linux, through GStreamer's `gst-launch-1.0`: VA-API (Intel, AMD) or
//! NVENC (NVIDIA), whichever works here (tried once per process, with a few test frames).
//!
//! Pictures go in as raw I420 on one pipe; encoded frames come out on another, framed with
//! GStreamer's data protocol (`gdppay`), so each read is exactly one frame. `gst-launch` can't
//! be told "key frame now", so a key frame request restarts the pipeline (its first frame is
//! one); that happens when a viewer starts or its decoder resets, not during normal use.
//!
//! Packages: Debian/Ubuntu `gstreamer1.0-tools gstreamer1.0-plugins-bad` (plus the VA driver,
//! e.g. `intel-media-va-driver` or `mesa-va-drivers`; NVIDIA's driver brings NVENC).

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};

use super::super::soft::{Bits, FrameEncoder, Spec, Yuv, has_idr};

/// An encoder element and how to set it up.
struct Candidate {
    /// For the log.
    name: &'static str,
    element: &'static str,
    /// Low-latency settings (property names differ between elements and GStreamer
    /// versions; without them it's tried with the bit rate alone).
    tuned: fn(&Spec) -> Vec<String>,
    bitrate: fn(&Spec) -> String,
}

fn kbps(s: &Spec) -> u32 {
    (s.enc.bitrate / 1000).max(100)
}

const CANDIDATES: &[Candidate] = &[
    Candidate {
        name: "VA-API",
        element: "vah264enc",
        tuned: |s| {
            vec![
                "rate-control=cbr".into(),
                "key-int-max=1024".into(),
                "b-frames=0".into(),
                format!("max-qp={}", s.max_qp),
            ]
        },
        bitrate: |s| format!("bitrate={}", kbps(s)),
    },
    Candidate {
        name: "VA-API (low power)",
        element: "vah264lpenc",
        tuned: |s| {
            vec![
                "rate-control=cbr".into(),
                "key-int-max=1024".into(),
                "b-frames=0".into(),
                format!("max-qp={}", s.max_qp),
            ]
        },
        bitrate: |s| format!("bitrate={}", kbps(s)),
    },
    Candidate {
        name: "NVENC",
        element: "nvh264enc",
        tuned: |s| {
            vec![
                "rc-mode=cbr".into(),
                "gop-size=-1".into(),
                "bframes=0".into(),
                "zerolatency=true".into(),
                format!("qp-max={}", s.max_qp),
            ]
        },
        bitrate: |s| format!("bitrate={}", kbps(s)),
    },
    Candidate {
        name: "VA-API (gstreamer-vaapi)",
        element: "vaapih264enc",
        tuned: |_| {
            vec![
                "rate-control=cbr".into(),
                "keyframe-period=3600".into(),
                "max-bframes=0".into(),
            ]
        },
        bitrate: |s| format!("bitrate={}", kbps(s)),
    },
];

/// The encoder that works here, and whether with its tuned settings.
#[derive(Clone, Copy)]
struct Choice {
    candidate: &'static Candidate,
    tuned: bool,
}

/// Runs `gst-launch-1.0` with `args` until it ends, at most `limit`. True if it succeeded.
fn runs(args: &[String], limit: Duration) -> bool {
    let Ok(mut child) = Command::new("gst-launch-1.0")
        .arg("-q")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let end = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Finds a working hardware encoder (once per process: it takes a second or two).
fn choice(spec: &Spec) -> Option<Choice> {
    static CHOICE: OnceLock<Option<Choice>> = OnceLock::new();
    *CHOICE.get_or_init(|| {
        let probe = |c: &'static Candidate, tuned: bool| {
            let mut args: Vec<String> = [
                "videotestsrc",
                "num-buffers=3",
                "!",
                "video/x-raw,format=NV12,width=640,height=480,framerate=30/1",
                "!",
                c.element,
            ]
            .map(String::from)
            .to_vec();
            args.push((c.bitrate)(spec));
            if tuned {
                args.extend((c.tuned)(spec));
            }
            args.extend(["!", "h264parse", "!", "fakesink"].map(String::from));
            runs(&args, Duration::from_secs(8))
        };
        for c in CANDIDATES {
            for tuned in [true, false] {
                if probe(c, tuned) {
                    tracing::info!(encoder = c.name, tuned, "hardware encoder found");
                    return Some(Choice {
                        candidate: c,
                        tuned,
                    });
                }
            }
        }
        tracing::info!("no hardware encoder (GStreamer with VA-API or NVENC): software");
        None
    })
}

/// The platform hook: a hardware encoder if one works here.
pub fn encoder(spec: &Spec) -> Option<Box<dyn FrameEncoder>> {
    let choice = choice(spec)?;
    match Gst::start(choice, spec) {
        Ok(g) => Some(Box::new(g)),
        Err(e) => {
            tracing::warn!(error = %e, "hardware encoder didn't start");
            None
        }
    }
}

// ---- GStreamer's data protocol (`gdppay`) ------------------------------------------------

/// Header: version (2 bytes), flags, padding, payload type (u16 BE), payload length (u32 BE),
/// timestamps and flags, two CRCs: 62 bytes.
const GDP_HEADER: usize = 62;
/// Payload type of a buffer (others are caps and events).
const GDP_BUFFER: u16 = 1;
/// Largest frame accepted (a 4K key frame is a few MB at most).
const MAX_FRAME: usize = 64 << 20;

/// Reads the next buffer's payload, skipping caps and events. `None` at the end.
fn read_gdp_buffer(r: &mut impl Read) -> anyhow::Result<Option<Vec<u8>>> {
    loop {
        let mut h = [0u8; GDP_HEADER];
        match r.read_exact(&mut h) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        if h[0] != 1 {
            bail!("unexpected data from GStreamer (version {}.{})", h[0], h[1]);
        }
        let kind = u16::from_be_bytes([h[4], h[5]]);
        let len = u32::from_be_bytes([h[6], h[7], h[8], h[9]]) as usize;
        if len > MAX_FRAME {
            bail!("a {len}-byte packet from GStreamer");
        }
        let mut payload = vec![0u8; len];
        r.read_exact(&mut payload)?;
        if kind == GDP_BUFFER {
            return Ok(Some(payload));
        }
    }
}

// ---- the pipeline ------------------------------------------------------------------------

/// How long one frame may take to come out before the encoder counts as stuck.
const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

struct Gst {
    choice: Choice,
    spec: Spec,
    child: Child,
    input: std::fs::File,
    output: Receiver<anyhow::Result<Vec<u8>>>,
    frame: Vec<u8>,
    /// Nothing encoded yet: the next frame is a key frame anyway.
    fresh: bool,
    name: String,
}

impl Gst {
    fn start(choice: Choice, spec: &Spec) -> anyhow::Result<Self> {
        let (w, h) = (spec.width, spec.height);
        let c = choice.candidate;
        let mut args: Vec<String> = vec![
            "-q".into(),
            "fdsrc".into(),
            "fd=3".into(),
            "!".into(),
            "rawvideoparse".into(),
            "format=i420".into(),
            format!("width={w}"),
            format!("height={h}"),
            format!("framerate={}/1", spec.enc.fps.max(1)),
            // Our planes are packed tightly (GStreamer would pad rows to 4 bytes).
            format!("plane-strides=<{w},{},{}>", w / 2, w / 2),
            format!("plane-offsets=<0,{},{}>", w * h, w * h + w * h / 4),
            "!".into(),
            "videoconvert".into(),
            "!".into(),
            "video/x-raw,format=NV12".into(),
            "!".into(),
            c.element.into(),
            (c.bitrate)(spec),
        ];
        if choice.tuned {
            args.extend((c.tuned)(spec));
        }
        args.extend(
            [
                "!",
                "h264parse",
                "config-interval=-1",
                "!",
                "video/x-h264,stream-format=byte-stream,alignment=au",
                "!",
                "gdppay",
                "!",
                "fdsink",
                "fd=4",
                "sync=false",
            ]
            .map(String::from),
        );

        let pipe = || -> anyhow::Result<(OwnedFd, OwnedFd)> {
            let mut fds = [0i32; 2];
            // SAFETY: a pipe; both ends are owned right away.
            if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
                bail!("Couldn't set up the encoder.");
            }
            // SAFETY: fresh descriptors from pipe2, owned once each.
            Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
        };
        let (in_read, in_write) = pipe()?;
        let (out_read, out_write) = pipe()?;
        let mut cmd = Command::new("gst-launch-1.0");
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let (r, wr) = (in_read.as_raw_fd(), out_write.as_raw_fd());
        // SAFETY: only async-signal-safe dup2 calls between fork and exec; dup2 clears
        // close-on-exec on the new descriptors 3 and 4.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(move || {
                if libc::dup2(r, 3) < 0 || libc::dup2(wr, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| anyhow!("gst-launch-1.0: {e}"))?;
        drop(in_read);
        drop(out_write);

        // Frames come out on their own thread, so a stuck encoder can be noticed.
        let (tx, output) = sync_channel(4);
        let mut reader = std::fs::File::from(out_read);
        std::thread::Builder::new()
            .name("gst-encoded".into())
            .spawn(move || {
                loop {
                    let next = match read_gdp_buffer(&mut reader) {
                        Ok(Some(f)) => Ok(f),
                        Ok(None) => Err(anyhow!("the encoder stopped")),
                        Err(e) => Err(e),
                    };
                    let end = next.is_err();
                    if tx.send(next).is_err() || end {
                        return;
                    }
                }
            })
            .context("encoder thread")?;
        Ok(Gst {
            choice,
            spec: *spec,
            child,
            input: std::fs::File::from(in_write),
            output,
            frame: vec![0u8; w * h * 3 / 2],
            fresh: true,
            name: format!("{} via GStreamer ({})", c.name, c.element),
        })
    }

    fn restart(&mut self) -> anyhow::Result<()> {
        let fresh = Gst::start(self.choice, &self.spec)?;
        *self = fresh;
        Ok(())
    }
}

impl FrameEncoder for Gst {
    fn encode(&mut self, yuv: &Yuv, key: bool, _at: Duration) -> anyhow::Result<Option<Bits>> {
        if key && !self.fresh {
            // A fresh pipeline starts with a key frame.
            self.restart()?;
        }
        self.fresh = false;
        let ys = yuv.width * yuv.height;
        let cs = ys / 4;
        self.frame[..ys].copy_from_slice(&yuv.y);
        self.frame[ys..ys + cs].copy_from_slice(&yuv.u);
        self.frame[ys + cs..ys + 2 * cs].copy_from_slice(&yuv.v);
        if let Err(e) = self.input.write_all(&self.frame) {
            let _ = self.restart();
            bail!("the encoder stopped taking pictures ({e})");
        }
        match self.output.recv_timeout(FRAME_TIMEOUT) {
            Ok(Ok(data)) => {
                let key = has_idr(&data);
                Ok(Some(Bits { data, key }))
            }
            Ok(Err(e)) => {
                let _ = self.restart();
                Err(e)
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = self.restart();
                bail!("the encoder took too long")
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _ = self.restart();
                bail!("the encoder stopped")
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for Gst {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; GDP_HEADER];
        h[0] = 1;
        h[4..6].copy_from_slice(&kind.to_be_bytes());
        h[6..10].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        h.extend_from_slice(payload);
        h
    }

    #[test]
    fn reads_buffers_and_skips_caps_and_events() {
        let mut stream = packet(2, b"video/x-h264");
        stream.extend(packet(64 + 5, b"segment"));
        stream.extend(packet(GDP_BUFFER, &[0, 0, 0, 1, 0x65, 9]));
        stream.extend(packet(GDP_BUFFER, &[0, 0, 0, 1, 0x41, 7]));
        let mut r = &stream[..];
        assert_eq!(
            read_gdp_buffer(&mut r).unwrap().unwrap(),
            vec![0, 0, 0, 1, 0x65, 9]
        );
        assert_eq!(
            read_gdp_buffer(&mut r).unwrap().unwrap(),
            vec![0, 0, 0, 1, 0x41, 7]
        );
        assert!(read_gdp_buffer(&mut r).unwrap().is_none());
    }

    #[test]
    fn rejects_other_data() {
        let mut stream = packet(GDP_BUFFER, b"x");
        stream[0] = b'G';
        assert!(read_gdp_buffer(&mut &stream[..]).is_err());
    }
}
