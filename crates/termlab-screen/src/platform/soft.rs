//! Capture and encoding for Windows and Linux: the platform code grabs the screen (BGRA),
//! this module scales it, converts it to YUV and encodes it with OpenH264 (Cisco's H.264
//! encoder, built in, with its screen content mode), with the same flow control as on the
//! Mac: at most [`WINDOW`] frames unacknowledged, a newer capture replacing the one waiting,
//! key frames on request, and a still screen re-encoded until it's sharp.
//!
//! Two threads per stream: capture (the platform's [`Grab`], polled at the frame rate; only
//! changed pictures go on) and encode.
//!
//! The encoder is the platform's hardware one where it has one that works (Windows: the GPU's
//! Media Foundation encoder; Linux: VA-API or NVENC through GStreamer), else OpenH264. Set
//! `TERMLAB_SCREEN_ENCODER=software` to always use OpenH264.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use openh264::OpenH264API;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, QpRange, RateControlMode,
    UsageType, VuiConfig,
};
use openh264::formats::YUVSource;
use tokio::sync::mpsc::UnboundedSender;

use super::{Bounds, REFINE_AFTER, REFINE_MAX, Video, WINDOW};
use crate::proto::{Encoded, Encoding, VideoConfig, avc_codec};

/// A captured picture: 4 bytes per pixel, blue, green, red, then anything.
pub struct Image {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Bytes per row.
    pub stride: usize,
}

/// A platform's screen grabber, made and used on the capture thread.
pub trait Grab {
    /// The current picture, or `None` when it can't be had right now (a locked screen, a
    /// mode change); the capture loop then tries again.
    fn grab(&mut self) -> anyhow::Result<Option<Image>>;
}

/// Reads the pointer apart from the picture (see [`super::cursor`]).
pub trait Pointer: Send {
    fn read(&mut self) -> Option<super::cursor::Cursor>;
}

/// What a platform opens for a stream: the grabber and where the display is.
pub struct Opened {
    pub grab: Box<dyn Grab>,
    /// Where the platform can read the pointer on its own.
    pub pointer: Option<Box<dyn Pointer>>,
    pub display: u32,
    pub bounds: Bounds,
    /// Pixel size of the display.
    pub width: u32,
    pub height: u32,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- BGRA → I420, scaled -----------------------------------------------------------

/// A YUV 4:2:0 picture (BT.709, limited range).
pub struct Yuv {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl YUVSource for Yuv {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    fn strides(&self) -> (usize, usize, usize) {
        (self.width, self.width / 2, self.width / 2)
    }
    fn y(&self) -> &[u8] {
        &self.y
    }
    fn u(&self) -> &[u8] {
        &self.u
    }
    fn v(&self) -> &[u8] {
        &self.v
    }
}

/// Source ranges for each output column or row: an area average, so text shrinks smoothly.
fn spans(src: usize, dst: usize) -> Vec<(usize, usize)> {
    (0..dst)
        .map(|i| {
            let a = i * src / dst;
            let b = ((i + 1) * src / dst).max(a + 1).min(src);
            (a, b)
        })
        .collect()
}

/// One output row as RGB: the area average of its source rows `y0..y1`.
fn row_rgb(img: &Image, xs: &[(usize, usize)], (y0, y1): (usize, usize), out: &mut [u8]) {
    let sw = img.width as usize;
    if xs.len() == sw && y1 == y0 + 1 {
        let row = &img.data[y0 * img.stride..y0 * img.stride + sw * 4];
        for (o, p) in out.chunks_exact_mut(3).zip(row.chunks_exact(4)) {
            o[0] = p[2];
            o[1] = p[1];
            o[2] = p[0];
        }
        return;
    }
    for (o, &(x0, x1)) in out.chunks_exact_mut(3).zip(xs) {
        let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
        for sy in y0..y1 {
            let row = &img.data[sy * img.stride + x0 * 4..sy * img.stride + x1 * 4];
            for p in row.chunks_exact(4) {
                b += u32::from(p[0]);
                g += u32::from(p[1]);
                r += u32::from(p[2]);
                n += 1;
            }
        }
        o[0] = (r / n) as u8;
        o[1] = (g / n) as u8;
        o[2] = (b / n) as u8;
    }
}

/// Converts a band of output rows (an even number) into its slices of the planes.
fn convert_band(
    img: &Image,
    xs: &[(usize, usize)],
    ys: &[(usize, usize)],
    y: &mut [u8],
    u: &mut [u8],
    v: &mut [u8],
) {
    let width = xs.len();
    let cw = width / 2;
    let (mut top, mut bottom) = (vec![0u8; width * 3], vec![0u8; width * 3]);
    for (pair, rows) in ys.chunks_exact(2).enumerate() {
        row_rgb(img, xs, rows[0], &mut top);
        row_rgb(img, xs, rows[1], &mut bottom);
        // BT.709 limited range, ×256.
        for (row, rgb) in [&top, &bottom].into_iter().enumerate() {
            let out = &mut y[(pair * 2 + row) * width..(pair * 2 + row + 1) * width];
            for (o, p) in out.iter_mut().zip(rgb.chunks_exact(3)) {
                let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
                *o = ((47 * r + 157 * g + 16 * b + 128) / 256 + 16) as u8;
            }
        }
        for cx in 0..cw {
            let i = cx * 6;
            let sum = |c: usize| {
                i32::from(top[i + c])
                    + i32::from(top[i + 3 + c])
                    + i32::from(bottom[i + c])
                    + i32::from(bottom[i + 3 + c])
            };
            let (r, g, b) = (sum(0) / 4, sum(1) / 4, sum(2) / 4);
            u[pair * cw + cx] =
                ((-26 * r - 87 * g + 112 * b + 128) / 256 + 128).clamp(0, 255) as u8;
            v[pair * cw + cx] =
                ((112 * r - 102 * g - 10 * b + 128) / 256 + 128).clamp(0, 255) as u8;
        }
    }
}

/// Scales `img` to `width`×`height` (both even) and converts it, on several threads (one
/// band of rows each).
pub fn to_yuv(img: &Image, width: usize, height: usize) -> Yuv {
    let xs = spans(img.width as usize, width);
    let ys = spans(img.height as usize, height);
    let mut yuv = Yuv {
        width,
        height,
        y: vec![0; width * height],
        u: vec![0; width * height / 4],
        v: vec![0; width * height / 4],
    };
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 8);
    let band = (height / 2).div_ceil(threads).max(1) * 2;
    let cw = width / 2;
    std::thread::scope(|scope| {
        let bands = yuv
            .y
            .chunks_mut(band * width)
            .zip(yuv.u.chunks_mut(band / 2 * cw))
            .zip(yuv.v.chunks_mut(band / 2 * cw))
            .zip(ys.chunks(band));
        for (((y, u), v), ys) in bands {
            let xs = &xs;
            scope.spawn(move || convert_band(img, xs, ys, y, u, v));
        }
    });
    yuv
}

/// Output size for a display of `w`×`h` pixels at most `max_width` wide, both even.
/// OpenH264 encodes up to 3840×2160 (or 2160×3840): larger pictures are scaled down to fit,
/// keeping the aspect (a 3840×2400 display goes out at 3456×2160).
pub fn fit(w: u32, h: u32, max_width: u32) -> (usize, usize) {
    let (w, h) = (f64::from(w.max(2)), f64::from(h.max(2)));
    let scale = [
        1.0,
        f64::from(max_width) / w,
        3840.0 / w.max(h),
        2160.0 / w.min(h),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min);
    let even = |v: f64| ((v * scale).floor() as u32).max(2) & !1;
    (even(w) as usize, even(h) as usize)
}

// ---- Annex B → AVCC ------------------------------------------------------------------

/// NAL units of an Annex B stream (start codes removed).
pub fn nal_units(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (n, &s) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(stream.len(), |&next| next - 3);
        // A 4-byte start code leaves a zero before the next 3-byte one.
        while end > s && stream[end - 1] == 0 && starts.get(n + 1).is_some() {
            end -= 1;
        }
        if end > s {
            out.push(&stream[s..end]);
        }
    }
    out
}

/// The `avcC` record for one SPS and one PPS.
pub fn avcc_record(sps: &[u8], pps: &[u8]) -> Option<Vec<u8>> {
    if sps.len() < 4 {
        return None;
    }
    let mut r = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe1];
    r.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    r.extend_from_slice(sps);
    r.push(1);
    r.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    r.extend_from_slice(pps);
    Some(r)
}

// ---- encoders ----------------------------------------------------------------------------

/// One encoded picture, Annex B.
pub struct Bits {
    pub data: Vec<u8>,
    pub key: bool,
}

/// Turns pictures into H.264 (Annex B), made and used on the encode thread.
pub trait FrameEncoder {
    /// `key`: make this a key frame. `Ok(None)`: nothing came out (a skipped frame).
    fn encode(&mut self, yuv: &Yuv, key: bool, at: Duration) -> anyhow::Result<Option<Bits>>;
    fn name(&self) -> &str;
}

/// What an encoder needs to know.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct Spec {
    pub width: usize,
    pub height: usize,
    pub enc: Encoding,
    /// The coarsest quantizer to allow (see [`max_qp`]).
    pub max_qp: u8,
}

/// A platform's hardware encoder, if it has one that works for `spec`.
pub type Hardware = fn(&Spec) -> Option<Box<dyn FrameEncoder>>;

/// No hardware encoder.
#[allow(dead_code)]
pub fn no_hardware(_: &Spec) -> Option<Box<dyn FrameEncoder>> {
    None
}

/// Hardware encoders allowed (not switched off with `TERMLAB_SCREEN_ENCODER=software`).
pub fn hardware_allowed() -> bool {
    std::env::var("TERMLAB_SCREEN_ENCODER").map_or(true, |v| v != "software")
}

/// OpenH264, in its screen content mode.
struct Software {
    encoder: Encoder,
}

impl Software {
    fn new(spec: &Spec) -> anyhow::Result<Self> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(spec.enc.bitrate))
            .max_frame_rate(FrameRate::from_hz(spec.enc.fps as f32))
            .skip_frames(false)
            .complexity(Complexity::Low)
            .qp(QpRange::new(12, spec.max_qp))
            .vui(VuiConfig::bt709());
        let encoder = Encoder::with_api_config(OpenH264API::from_source(), config)
            .context("The video encoder couldn't start.")?;
        Ok(Software { encoder })
    }
}

impl FrameEncoder for Software {
    fn encode(&mut self, yuv: &Yuv, key: bool, at: Duration) -> anyhow::Result<Option<Bits>> {
        if key {
            self.encoder.force_intra_frame();
        }
        let bits = self
            .encoder
            .encode_at(yuv, openh264::Timestamp::from_millis(at.as_millis() as u64))?;
        let kind = bits.frame_type();
        if matches!(kind, FrameType::Skip | FrameType::Invalid) {
            return Ok(None);
        }
        let data = bits.to_vec();
        Ok((!data.is_empty()).then_some(Bits {
            data,
            key: matches!(kind, FrameType::IDR),
        }))
    }

    fn name(&self) -> &str {
        "OpenH264 (software)"
    }
}

/// Is there an IDR slice (a key frame) in this Annex B access unit?
#[allow(dead_code)]
pub fn has_idr(stream: &[u8]) -> bool {
    nal_units(stream).iter().any(|n| n[0] & 0x1f == 5)
}

/// The picture as NV12 (the Y plane, then U and V interleaved), what hardware encoders take.
#[allow(dead_code)]
pub fn write_nv12(yuv: &Yuv, out: &mut [u8]) {
    let ys = yuv.width * yuv.height;
    out[..ys].copy_from_slice(&yuv.y);
    for (o, (u, v)) in out[ys..ys + ys / 2]
        .chunks_exact_mut(2)
        .zip(yuv.u.iter().zip(&yuv.v))
    {
        o[0] = *u;
        o[1] = *v;
    }
}

// ---- the encoder thread ----------------------------------------------------------------

#[derive(Default)]
struct Job {
    /// The newest capture not yet encoded (converted on the capture thread, so converting
    /// and encoding overlap).
    pending: Option<Yuv>,
    /// Encode the last picture again (refining a still screen, or a key frame on request).
    again: bool,
    key: bool,
    stop: bool,
}

struct Shared {
    job: Mutex<Job>,
    wake: Condvar,
    inflight: AtomicU32,
    last_size: AtomicUsize,
    refined: AtomicU32,
    encoded_at: Mutex<Instant>,
    /// Why the capture gave up, if it did.
    failed: Mutex<Option<String>>,
}

/// Capture errors in a row (200 ms apart) before giving up: 5 seconds.
const GIVE_UP_AFTER: u32 = if cfg!(test) { 3 } else { 25 };

impl Shared {
    fn sent_one_less(&self) {
        let _ = self
            .inflight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
    }
}

/// Hardware encoding errors in a row before switching to OpenH264 for the rest of the stream.
const HARDWARE_GIVE_UP: u32 = 3;

struct EncodeLoop {
    shared: Arc<Shared>,
    out: UnboundedSender<Encoded>,
    width: usize,
    height: usize,
    spec: Spec,
    hardware: Hardware,
    last: Option<Yuv>,
    sent_config: Option<VideoConfig>,
    started: Instant,
}

impl EncodeLoop {
    fn encoder(&self) -> Option<Box<dyn FrameEncoder>> {
        if hardware_allowed()
            && let Some(e) = (self.hardware)(&self.spec)
        {
            return Some(e);
        }
        match Software::new(&self.spec) {
            Ok(e) => Some(Box::new(e)),
            Err(e) => {
                *lock(&self.shared.failed) = Some(e.to_string());
                None
            }
        }
    }

    fn run(mut self) {
        let Some(mut encoder) = self.encoder() else {
            return;
        };
        tracing::info!(encoder = encoder.name(), "encoding");
        let mut errors = 0u32;
        loop {
            let job = {
                let mut j = lock(&self.shared.job);
                loop {
                    if j.stop {
                        return;
                    }
                    let room = self.shared.inflight.load(Ordering::Acquire) < WINDOW;
                    if room && (j.pending.is_some() || (j.again && self.last.is_some())) {
                        break;
                    }
                    j = self
                        .shared
                        .wake
                        .wait_timeout(j, Duration::from_millis(500))
                        .map(|(g, _)| g)
                        .unwrap_or_else(|e| e.into_inner().0);
                }
                std::mem::take(&mut *j)
            };
            if let Some(yuv) = job.pending {
                self.last = Some(yuv);
            }
            let Some(yuv) = &self.last else { continue };
            *lock(&self.shared.encoded_at) = Instant::now();
            let ts = self.started.elapsed();
            let bits = match encoder.encode(yuv, job.key, ts) {
                Ok(b) => {
                    errors = 0;
                    b
                }
                Err(e) => {
                    tracing::warn!(error = %e, encoder = encoder.name(), "encode failed");
                    errors += 1;
                    // A hardware encoder that keeps failing (a driver reset, …): software.
                    if errors >= HARDWARE_GIVE_UP
                        && encoder.name() != "OpenH264 (software)"
                        && let Ok(soft) = Software::new(&self.spec)
                    {
                        tracing::warn!("switching to the software encoder");
                        encoder = Box::new(soft);
                        errors = 0;
                        let mut j = lock(&self.shared.job);
                        j.key = true;
                        j.again = true;
                        continue;
                    }
                    if job.key {
                        lock(&self.shared.job).key = true;
                    }
                    continue;
                }
            };
            let Some(Bits { data: stream, key }) = bits else {
                self.shared.last_size.store(0, Ordering::Release);
                continue;
            };
            let (mut sps, mut pps) = (None, None);
            let mut data = Vec::with_capacity(stream.len());
            for nal in nal_units(&stream) {
                match nal[0] & 0x1f {
                    7 => sps = Some(nal),
                    8 => pps = Some(nal),
                    _ => {
                        data.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                        data.extend_from_slice(nal);
                    }
                }
            }
            self.shared.last_size.store(data.len(), Ordering::Release);
            let mut config = None;
            if let (Some(sps), Some(pps)) = (sps, pps)
                && let Some(record) = avcc_record(sps, pps)
                && let Some(codec) = avc_codec(&record)
            {
                let next = VideoConfig {
                    codec,
                    description: record,
                    width: self.width as u32,
                    height: self.height as u32,
                };
                if self.sent_config.as_ref() != Some(&next) {
                    self.sent_config = Some(next.clone());
                    config = Some(next);
                }
            }
            if data.is_empty() {
                continue;
            }
            self.shared.inflight.fetch_add(1, Ordering::AcqRel);
            let frame = Encoded {
                key,
                ts_us: ts.as_micros() as u64,
                data,
                config,
            };
            if self.out.send(frame).is_err() {
                self.shared.sent_one_less();
            }
        }
    }
}

/// The coarsest quantizer the rate control may use. Without a cap it keeps key frames (and
/// the first frame after any movement) soft to stay on budget: measured on a 1080p
/// screenshot of a code editor, a key frame came out at 38.4 dB (40.8 after 8 refining
/// passes); capped at 28 it's 42.0 dB (44.8), visibly crisper text, for 30% more bytes on
/// that frame and about twice the bytes while scrolling. The window then trades frames for
/// sharpness rather than adding delay.
fn max_qp(q: crate::proto::Quality) -> u8 {
    use crate::proto::Quality;
    match q {
        Quality::Low => 32,
        Quality::Balanced => 28,
        Quality::High | Quality::Native => 26,
    }
}

// ---- the stream ------------------------------------------------------------------------

/// One display being captured and encoded.
pub struct Stream {
    shared: Arc<Shared>,
    pointer: Mutex<Option<Box<dyn Pointer>>>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    display: u32,
    bounds: Bounds,
    settled_bytes: usize,
}

impl Stream {
    /// `open` runs on the capture thread (some platforms' handles belong to the thread that
    /// made them).
    pub fn start(
        enc: Encoding,
        out: UnboundedSender<Encoded>,
        hardware: Hardware,
        open: impl FnOnce() -> anyhow::Result<Opened> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let shared = Arc::new(Shared {
            job: Mutex::new(Job {
                key: true,
                ..Job::default()
            }),
            wake: Condvar::new(),
            inflight: AtomicU32::new(0),
            last_size: AtomicUsize::new(0),
            refined: AtomicU32::new(0),
            encoded_at: Mutex::new(Instant::now()),
            failed: Mutex::new(None),
        });
        let stop = Arc::new(AtomicBool::new(false));
        type Ready = (u32, Bounds, u32, u32, Option<Box<dyn Pointer>>);
        let (ready_tx, ready) = sync_channel::<anyhow::Result<Ready>>(1);

        let capture = {
            let shared = shared.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("screen-capture".into())
                .spawn(move || {
                    let mut opened = match open() {
                        Ok(o) => o,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok((
                        opened.display,
                        opened.bounds,
                        opened.width,
                        opened.height,
                        opened.pointer.take(),
                    )));
                    let size = fit(opened.width, opened.height, enc.max_width);
                    capture_loop(&mut *opened.grab, size, enc.fps, &shared, &stop);
                })?
        };
        let (display, bounds, w, h, pointer) = match ready.recv_timeout(Duration::from_secs(90)) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                let _ = capture.join();
                return Err(e);
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                return Err(anyhow!("The screen capture didn't start."));
            }
        };

        let (width, height) = fit(w, h, enc.max_width);
        let spec = Spec {
            width,
            height,
            enc,
            max_qp: max_qp(enc.quality),
        };
        let encode = {
            let looper = EncodeLoop {
                shared: shared.clone(),
                out,
                width,
                height,
                spec,
                hardware,
                last: None,
                sent_config: None,
                started: Instant::now(),
            };
            std::thread::Builder::new()
                .name("screen-encode".into())
                .spawn(move || looper.run())?
        };
        let id = display;
        tracing::info!(id, w, h, width, height, "capturing");
        Ok(Stream {
            shared,
            pointer: Mutex::new(pointer),
            stop,
            threads: vec![capture, encode],
            display,
            bounds,
            // About 5 KB at 4K, 1.4 KB at 1080p.
            settled_bytes: (width * height / 1500).max(512),
        })
    }
}

impl Video for Stream {
    fn display(&self) -> u32 {
        self.display
    }

    fn bounds(&self) -> Bounds {
        self.bounds
    }

    fn ack(&self) {
        self.shared.sent_one_less();
        self.shared.wake.notify_one();
    }

    fn keyframe(&self) {
        let mut j = lock(&self.shared.job);
        j.key = true;
        j.again = true;
        self.shared.refined.store(0, Ordering::Release);
        self.shared.wake.notify_one();
    }

    /// One more pass over a still screen, if it needs one (see the Mac's stream).
    fn refine(&self) {
        let s = &self.shared;
        if s.inflight.load(Ordering::Acquire) > 0
            || s.refined.load(Ordering::Acquire) >= REFINE_MAX
            || s.last_size.load(Ordering::Acquire) < self.settled_bytes
            || lock(&s.encoded_at).elapsed() < REFINE_AFTER
        {
            return;
        }
        let mut j = lock(&s.job);
        if j.pending.is_some() {
            return;
        }
        j.again = true;
        s.refined.fetch_add(1, Ordering::AcqRel);
        s.wake.notify_one();
    }

    fn reset_window(&self) {
        self.shared.inflight.store(0, Ordering::Release);
        self.shared.wake.notify_one();
    }

    fn inflight(&self) -> u32 {
        self.shared.inflight.load(Ordering::Acquire)
    }

    fn failed(&self) -> Option<String> {
        lock(&self.shared.failed).clone()
    }

    fn cursor(&self) -> Option<super::cursor::Cursor> {
        lock(&self.pointer).as_mut()?.read()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        lock(&self.shared.job).stop = true;
        self.shared.wake.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Grabs at the frame rate; changed pictures go to the encoder, converted to `size`.
fn capture_loop(
    grab: &mut dyn Grab,
    size: (usize, usize),
    fps: u32,
    shared: &Shared,
    stop: &AtomicBool,
) {
    let every = Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
    let mut previous: Option<Image> = None;
    let mut failures = 0u32;
    while !stop.load(Ordering::Acquire) {
        let started = Instant::now();
        match grab.grab() {
            Ok(Some(img)) => {
                failures = 0;
                let changed = previous.as_ref().is_none_or(|p| {
                    p.width != img.width || p.height != img.height || p.data != img.data
                });
                if changed {
                    let yuv = to_yuv(&img, size.0, size.1);
                    shared.refined.store(0, Ordering::Release);
                    lock(&shared.job).pending = Some(yuv);
                    shared.wake.notify_one();
                    previous = Some(img);
                }
            }
            Ok(None) => {}
            Err(e) => {
                failures += 1;
                if failures == 1 {
                    tracing::warn!(error = %e, "capture failed");
                }
                if failures >= GIVE_UP_AFTER {
                    tracing::warn!(error = %e, "capture gave up");
                    *lock(&shared.failed) = Some(e.to_string());
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        if let Some(left) = every.checked_sub(started.elapsed()) {
            std::thread::sleep(left);
        }
    }
}

/// Development: a moving test pattern with text-like detail, for trying the encoder on a Mac.
#[cfg(feature = "soft")]
pub fn test_pattern() -> Opened {
    struct Pattern {
        n: u32,
    }
    impl Grab for Pattern {
        fn grab(&mut self) -> anyhow::Result<Option<Image>> {
            let (w, h) = (1920u32, 1080u32);
            self.n += 1;
            let mut data = vec![0u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    // Fine stripes (like text), a gradient, and a box that moves.
                    let stripe = if (x / 2 + y / 3) % 7 == 0 { 230 } else { 30 };
                    let moving = (x + self.n * 8) % w < 200 && y > 400 && y < 600;
                    data[i] = if moving { 40 } else { stripe };
                    data[i + 1] = (y * 255 / h) as u8;
                    data[i + 2] = if moving { 220 } else { (x * 255 / w) as u8 };
                    data[i + 3] = 255;
                }
            }
            Ok(Some(Image {
                data,
                width: w,
                height: h,
                stride: (w * 4) as usize,
            }))
        }
    }
    Opened {
        grab: Box::new(Pattern { n: 0 }),
        pointer: None,
        display: 1,
        bounds: Bounds {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        },
        width: 1920,
        height: 1080,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, bgra: [u8; 4]) -> Image {
        Image {
            data: bgra.repeat((w * h) as usize),
            width: w,
            height: h,
            stride: w as usize * 4,
        }
    }

    #[test]
    fn colors_convert_to_limited_range_bt709() {
        let white = to_yuv(&solid(4, 4, [255, 255, 255, 255]), 4, 4);
        assert_eq!((white.y[0], white.u[0], white.v[0]), (235, 128, 128));
        let black = to_yuv(&solid(4, 4, [0, 0, 0, 255]), 2, 2);
        assert_eq!((black.y[0], black.u[0], black.v[0]), (16, 128, 128));
        let red = to_yuv(&solid(2, 2, [0, 0, 255, 255]), 2, 2);
        assert!(
            red.v[0] > 230 && red.u[0] < 110,
            "{} {}",
            red.u[0],
            red.v[0]
        );
    }

    #[test]
    fn scaling_keeps_the_aspect_and_even_sizes() {
        assert_eq!(fit(3840, 2160, 1920), (1920, 1080));
        assert_eq!(fit(1366, 768, 3840), (1366, 768));
        assert_eq!(fit(1365, 767, 3840), (1364, 766));
        // Within OpenH264's limits whatever the shape.
        assert_eq!(fit(3840, 2400, 3840), (3456, 2160));
        assert_eq!(fit(5120, 1440, 3840), (3840, 1080));
        assert_eq!(fit(2160, 3840, 3840), (2160, 3840));
        assert_eq!(fit(6016, 3384, 3840), (3840, 2160));
        assert_eq!(spans(4, 2), vec![(0, 2), (2, 4)]);
        assert_eq!(spans(2, 4), vec![(0, 1), (0, 1), (1, 2), (1, 2)]);
    }

    #[test]
    fn annex_b_splits_into_nal_units() {
        let s = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4, 0,
        ];
        assert_eq!(
            nal_units(&s),
            vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 4, 0][..]]
        );
        let r = avcc_record(&[0x67, 0x42, 0xc0, 0x1f], &[0x68]).unwrap();
        assert_eq!(avc_codec(&r).as_deref(), Some("avc1.42c01f"));
    }

    #[test]
    fn a_capture_that_keeps_failing_ends_the_stream() {
        struct Broken;
        impl Grab for Broken {
            fn grab(&mut self) -> anyhow::Result<Option<Image>> {
                anyhow::bail!("the display went away")
            }
        }
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let enc = Encoding {
            codec: crate::proto::Codec::H264,
            quality: crate::proto::Quality::Balanced,
            max_width: 1920,
            fps: 30,
            bitrate: 1_000_000,
            chroma444: false,
            draw_cursor: true,
        };
        let stream = Stream::start(enc, tx, no_hardware, || {
            Ok(Opened {
                grab: Box::new(Broken),
                pointer: None,
                display: 1,
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    w: 64.0,
                    h: 64.0,
                },
                width: 64,
                height: 64,
            })
        })
        .unwrap();
        assert_eq!(stream.failed(), None);
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(stream.failed().as_deref(), Some("the display went away"));
    }

    #[test]
    fn encodes_a_picture_and_sends_the_config_first() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let open = || {
            struct Still;
            impl Grab for Still {
                fn grab(&mut self) -> anyhow::Result<Option<Image>> {
                    let mut img = solid(320, 240, [30, 60, 90, 255]);
                    for x in 0..100 {
                        img.data[(120 * 320 + x) * 4..][..3].copy_from_slice(&[255, 255, 255]);
                    }
                    Ok(Some(img))
                }
            }
            Ok(Opened {
                grab: Box::new(Still),
                pointer: None,
                display: 1,
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    w: 320.0,
                    h: 240.0,
                },
                width: 320,
                height: 240,
            })
        };
        let enc = Encoding {
            codec: crate::proto::Codec::H264,
            quality: crate::proto::Quality::Balanced,
            max_width: 1920,
            fps: 30,
            bitrate: 1_000_000,
            chroma444: false,
            draw_cursor: true,
        };
        let stream = Stream::start(enc, tx, no_hardware, open).unwrap();
        let first = rx.blocking_recv().unwrap();
        assert!(first.key);
        let cfg = first.config.unwrap();
        assert!(cfg.codec.starts_with("avc1.42"), "{}", cfg.codec);
        assert_eq!((cfg.width, cfg.height), (320, 240));
        // AVCC: a 4-byte length, then an IDR slice.
        assert_eq!(first.data[4] & 0x1f, 5);
        stream.ack();
        stream.keyframe();
        let again = rx.blocking_recv().unwrap();
        assert!(again.key);
        assert!(again.config.is_none(), "same config isn't sent twice");
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `cargo test --release --features soft -- --ignored bench --nocapture`
    #[test]
    #[ignore]
    fn bench_4k() {
        let (w, h) = (3840u32, 2160u32);
        let mut img = Image {
            data: vec![0u8; (w * h * 4) as usize],
            width: w,
            height: h,
            stride: (w * 4) as usize,
        };
        for (i, p) in img.data.chunks_exact_mut(4).enumerate() {
            p[0] = (i % 251) as u8;
            p[1] = (i / 3840 % 255) as u8;
            p[2] = if (i % 7) == 0 { 230 } else { 20 };
        }
        for (ow, oh) in [(3840usize, 2160usize), (2560, 1440), (1920, 1080)] {
            let t = Instant::now();
            let yuv = to_yuv(&img, ow, oh);
            let conv = t.elapsed();
            let config = EncoderConfig::new()
                .usage_type(UsageType::ScreenContentRealTime)
                .rate_control_mode(RateControlMode::Bitrate)
                .bitrate(BitRate::from_bps(16_000_000))
                .max_frame_rate(FrameRate::from_hz(30.0))
                .skip_frames(false)
                .complexity(Complexity::Low);
            let mut enc = Encoder::with_api_config(OpenH264API::from_source(), config).unwrap();
            let t = Instant::now();
            let _ = enc.encode(&yuv).unwrap().to_vec();
            let first = t.elapsed();
            // Small changes per frame, like typing.
            let mut y = yuv;
            let t = Instant::now();
            for n in 0..10 {
                y.y[n * 1000 + 500] ^= 0xff;
                let _ = enc.encode(&y).unwrap().to_vec();
            }
            println!(
                "{ow}x{oh}: convert {conv:?}, key {first:?}, delta {:?}",
                t.elapsed() / 10
            );
        }
    }
}

#[cfg(test)]
mod quality {
    use super::*;
    use openh264::decoder::Decoder;

    fn load_bmp(path: &str) -> Image {
        let d = std::fs::read(path).unwrap();
        let off = u32::from_le_bytes(d[10..14].try_into().unwrap()) as usize;
        let w = i32::from_le_bytes(d[18..22].try_into().unwrap());
        let h = i32::from_le_bytes(d[22..26].try_into().unwrap());
        assert!(h < 0, "top-down only");
        let (w, h) = (w as u32, (-h) as u32);
        Image {
            data: d[off..off + (w * h * 4) as usize].to_vec(),
            width: w,
            height: h,
            stride: w as usize * 4,
        }
    }

    fn psnr(a: &[u8], b: &[u8], w: usize, h: usize, stride: usize) -> f64 {
        let mut se = 0f64;
        for y in 0..h {
            for x in 0..w {
                let d = f64::from(a[y * w + x]) - f64::from(b[y * stride + x]);
                se += d * d;
            }
        }
        let mse = se / (w * h) as f64;
        10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
    }

    pub fn run(label: &str, cfg: EncoderConfig, yuv: &Yuv, passes: usize) {
        let mut enc = Encoder::with_api_config(OpenH264API::from_source(), cfg).unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut line = format!("{label:28}");
        let t = Instant::now();
        for pass in 0..=passes {
            let bits = enc.encode(yuv).unwrap();
            let kind = bits.frame_type();
            let s = bits.to_vec();
            let q = dec
                .decode(&s)
                .ok()
                .flatten()
                .map(|d| psnr(&yuv.y, d.y(), yuv.width, yuv.height, d.strides().0))
                .unwrap_or(0.0);
            if pass < 3 || pass == passes {
                line += &format!(
                    " | {}{:>4}K {:.1}dB",
                    if matches!(kind, FrameType::IDR) {
                        "I"
                    } else {
                        ""
                    },
                    s.len() / 1024,
                    q
                );
            }
        }
        println!("{line}  ({:?}/frame)", t.elapsed() / (passes as u32 + 1));
    }

    /// Scrolling: the picture moves up 24 rows a frame (Y only shifted; enough for sizes).
    fn scroll(label: &str, cfg: EncoderConfig, yuv: &Yuv) {
        let mut enc = Encoder::with_api_config(OpenH264API::from_source(), cfg).unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut y = Yuv {
            width: yuv.width,
            height: yuv.height,
            y: yuv.y.clone(),
            u: yuv.u.clone(),
            v: yuv.v.clone(),
        };
        let first = enc.encode(&y).unwrap().to_vec();
        let _ = dec.decode(&first);
        let (mut bytes, mut q) = (0usize, 0f64);
        let n = 20;
        let t = Instant::now();
        for _ in 0..n {
            let w = y.width;
            y.y.rotate_left(24 * w);
            let s = enc.encode(&y).unwrap().to_vec();
            bytes += s.len();
            if let Some(d) = dec.decode(&s).ok().flatten() {
                q += psnr(&y.y, d.y(), y.width, y.height, d.strides().0);
            }
        }
        println!(
            "{:28} scroll: {:>4}K/frame {:.1}dB ({:?}/frame with decoding)",
            format!("  {label}"),
            bytes / n / 1024,
            q / n as f64,
            t.elapsed() / n as u32
        );
    }

    /// SHOT=screenshot.bmp cargo test --release --features soft -- --ignored quality --nocapture
    #[test]
    #[ignore]
    fn quality_on_a_screenshot() {
        let Ok(path) = std::env::var("SHOT") else {
            return;
        };
        let img = load_bmp(&path);
        use crate::proto::Quality;
        for (q, w, h, bps) in [
            (Quality::Balanced, 1920usize, 1080usize, 6_000_000u32),
            (Quality::High, 3840, 2160, 16_000_000),
        ] {
            let t = Instant::now();
            let yuv = to_yuv(&img, w, h);
            println!(
                "== {w}x{h} at {} Mbit/s (converted in {:?})",
                bps / 1_000_000,
                t.elapsed()
            );
            let base = EncoderConfig::new()
                .usage_type(UsageType::ScreenContentRealTime)
                .rate_control_mode(RateControlMode::Bitrate)
                .bitrate(BitRate::from_bps(bps))
                .max_frame_rate(FrameRate::from_hz(60.0))
                .skip_frames(false)
                .complexity(Complexity::Low)
                .vui(VuiConfig::bt709());
            for (label, cfg) in [
                ("no quantizer cap", base),
                ("capped (as used)", base.qp(QpRange::new(12, max_qp(q)))),
            ] {
                run(label, cfg, &yuv, 8);
                scroll(label, cfg, &yuv);
            }
        }
    }
}
