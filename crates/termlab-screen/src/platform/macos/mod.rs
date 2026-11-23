//! macOS: ScreenCaptureKit captures a display, VideoToolbox encodes it to H.264 in hardware,
//! and CGEvent posts mouse and keyboard input.
//!
//! These are C and Objective-C frameworks, so this module calls them through FFI; that is the
//! one place `unsafe` is allowed in this crate (the workspace denies it elsewhere).
#![allow(unsafe_code)]

mod input;
mod keymap;

pub use input::Input;

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType,
};
use objc2_core_graphics::{
    CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayIsBuiltin, CGDisplayMode, CGMainDisplayID,
    CGPreflightScreenCaptureAccess,
};
use objc2_core_media::{
    CMSampleBuffer, CMTime, kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms,
    kCMSampleAttachmentKey_NotSync, kCMTimeInvalid, kCMVideoCodecType_H264,
};
use objc2_core_video::{CVImageBuffer, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCFrameStatus, SCShareableContent, SCStream, SCStreamConfiguration,
    SCStreamFrameInfoStatus, SCStreamOutput, SCStreamOutputType,
};
use objc2_video_toolbox::{
    VTCompressionSession, VTEncodeInfoFlags, VTSession, VTSessionSetProperty,
    kVTCompressionPropertyKey_AllowFrameReordering, kVTCompressionPropertyKey_AverageBitRate,
    kVTCompressionPropertyKey_ExpectedFrameRate, kVTCompressionPropertyKey_MaxKeyFrameInterval,
    kVTCompressionPropertyKey_ProfileLevel, kVTCompressionPropertyKey_RealTime,
    kVTEncodeFrameOptionKey_ForceKeyFrame, kVTProfileLevel_H264_High_AutoLevel,
    kVTVideoEncoderSpecification_EnableLowLatencyRateControl,
};
use tokio::sync::mpsc::UnboundedSender;

use super::{Permissions, WINDOW};
use crate::proto::{DisplayInfo, Encoded, Quality, VideoConfig, avc_codec};

pub fn permissions() -> Permissions {
    Permissions {
        screen: CGPreflightScreenCaptureAccess(),
        // SAFETY: no arguments; reads this process's Accessibility trust.
        input: unsafe { objc2_application_services::AXIsProcessTrusted() },
    }
}

/// Carries a value across threads. The wrapped Objective-C and CoreFoundation objects are
/// thread-safe reference-counted objects; their types just aren't marked `Send`.
struct SendBox<T>(T);
// SAFETY: see above; each use hands the value to one other thread at a time.
unsafe impl<T> Send for SendBox<T> {}
// SAFETY: CVImageBuffers are immutable once captured; only their reference count changes.
unsafe impl<T> Sync for SendBox<T> {}

fn error_text(err: *mut NSError, fallback: &str) -> String {
    // SAFETY: the framework passes a valid NSError or null.
    unsafe { err.as_ref() }
        .map(|e| e.localizedDescription().to_string())
        .unwrap_or_else(|| fallback.to_string())
}

/// What can be captured now (displays, windows). Waits for ScreenCaptureKit's answer.
fn shareable() -> anyhow::Result<Retained<SCShareableContent>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        // SAFETY: a valid object (or null) handed to this callback; retained to keep it.
        let r = match unsafe { Retained::retain(content) } {
            Some(c) => Ok(SendBox(c)),
            None => Err(error_text(err, "The screen can't be captured.")),
        };
        let _ = tx.send(r);
    });
    // SAFETY: the block lives until the call completes; it only sends on a channel.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok(c)) => Ok(c.0),
        Ok(Err(msg)) => bail!(msg),
        Err(_) => bail!("The screen didn't answer (ScreenCaptureKit timed out)."),
    }
}

/// Physical pixels per point on a display.
fn display_scale(id: u32) -> f64 {
    let Some(mode) = CGDisplayCopyDisplayMode(id) else {
        return 1.0;
    };
    let points = CGDisplayMode::width(Some(&mode));
    let pixels = CGDisplayMode::pixel_width(Some(&mode));
    if points == 0 {
        1.0
    } else {
        (pixels as f64 / points as f64).max(1.0)
    }
}

fn info(d: &SCDisplay, index: usize, main: u32) -> DisplayInfo {
    // SAFETY: plain getters on a valid SCDisplay.
    let (id, w, h) = unsafe { (d.displayID(), d.width(), d.height()) };
    DisplayInfo {
        id,
        name: if CGDisplayIsBuiltin(id) {
            "Built-in display".into()
        } else {
            format!("Display {}", index + 1)
        },
        width: w.max(0) as u32,
        height: h.max(0) as u32,
        scale: display_scale(id),
        primary: id == main,
    }
}

pub fn displays() -> anyhow::Result<Vec<DisplayInfo>> {
    let content = shareable()?;
    let main = CGMainDisplayID();
    // SAFETY: a getter on a valid SCShareableContent.
    let list = unsafe { content.displays() };
    let mut out: Vec<DisplayInfo> = list
        .iter()
        .enumerate()
        .map(|(i, d)| info(&d, i, main))
        .collect();
    out.sort_by_key(|d| !d.primary);
    Ok(out)
}

// ---- the encoder ----------------------------------------------------------------------

/// Where encoded frames go; the encoder's output callback gets a pointer to it.
struct Sink {
    out: UnboundedSender<Encoded>,
    inflight: Arc<AtomicU32>,
    /// Size of the last encoded frame, to tell when refining a still screen is done.
    last_size: Arc<AtomicUsize>,
    width: u32,
    height: u32,
    sent_config: Mutex<Option<VideoConfig>>,
}

impl Sink {
    /// A frame that won't reach the browser: free its place in the window.
    fn dropped(&self) {
        let _ = self
            .inflight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
    }
}

struct Encoder {
    session: CFRetained<VTCompressionSession>,
    sink: *const Sink,
}

// SAFETY: VTCompressionSession may be used from any thread (calls are serialised by the
// pipeline's mutex); `sink` is an Arc kept alive until the session is invalidated.
unsafe impl Send for Encoder {}

fn cf<T: AsRef<CFType> + ?Sized>(v: &T) -> &CFType {
    v.as_ref()
}

fn set_property(session: &VTCompressionSession, key: &CFString, value: &CFType) {
    // SAFETY: a VTCompressionSession is a VTSession; key and value are valid CF objects.
    let status = unsafe {
        let s = &*(session as *const VTCompressionSession).cast::<VTSession>();
        VTSessionSetProperty(s, key, Some(value))
    };
    if status != 0 {
        tracing::debug!(status, key = %key, "encoder property not taken");
    }
}

impl Encoder {
    fn new(
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        sink: Arc<Sink>,
    ) -> anyhow::Result<Self> {
        let sink = Arc::into_raw(sink);
        let create = |low_latency: bool| -> Option<CFRetained<VTCompressionSession>> {
            // SAFETY: the constant is a valid CFString.
            let key = unsafe { kVTVideoEncoderSpecification_EnableLowLatencyRateControl };
            let spec =
                CFDictionary::<CFString, CFType>::from_slices(&[key], &[cf(CFBoolean::new(true))]);
            let mut out: *mut VTCompressionSession = std::ptr::null_mut();
            // SAFETY: valid arguments; `out` receives a +1 session on success. The callback
            // gets `sink`, which outlives the session (freed after invalidation).
            let status = unsafe {
                VTCompressionSession::create(
                    None,
                    width as i32,
                    height as i32,
                    kCMVideoCodecType_H264,
                    low_latency.then(|| spec.as_opaque()),
                    None,
                    None,
                    Some(on_encoded),
                    sink as *mut c_void,
                    NonNull::from(&mut out),
                )
            };
            if status != 0 {
                tracing::debug!(status, low_latency, "VTCompressionSessionCreate failed");
                return None;
            }
            // SAFETY: created with +1 retain count.
            NonNull::new(out).map(|p| unsafe { CFRetained::from_raw(p) })
        };
        let Some(session) = create(true).or_else(|| create(false)) else {
            // SAFETY: from Arc::into_raw above, not handed out.
            drop(unsafe { Arc::from_raw(sink) });
            bail!("The video encoder couldn't start.");
        };
        // SAFETY: the property keys are valid CFStrings.
        unsafe {
            set_property(
                &session,
                kVTCompressionPropertyKey_RealTime,
                cf(CFBoolean::new(true)),
            );
            set_property(
                &session,
                kVTCompressionPropertyKey_AllowFrameReordering,
                cf(CFBoolean::new(false)),
            );
            set_property(
                &session,
                kVTCompressionPropertyKey_ProfileLevel,
                cf(kVTProfileLevel_H264_High_AutoLevel),
            );
            // Key frames when asked for (a new or reset decoder), not on a timer.
            set_property(
                &session,
                kVTCompressionPropertyKey_MaxKeyFrameInterval,
                cf(&*CFNumber::new_i32(100_000)),
            );
            set_property(
                &session,
                kVTCompressionPropertyKey_AverageBitRate,
                cf(&*CFNumber::new_i32(bitrate as i32)),
            );
            set_property(
                &session,
                kVTCompressionPropertyKey_ExpectedFrameRate,
                cf(&*CFNumber::new_i32(fps as i32)),
            );
            session.prepare_to_encode_frames();
        }
        Ok(Encoder { session, sink })
    }

    fn encode(&self, buf: &CVImageBuffer, at: Duration, key: bool) -> bool {
        let pts = {
            // SAFETY: plain constructor.
            unsafe { CMTime::new(at.as_micros() as i64, 1_000_000) }
        };
        let props = key.then(|| {
            // SAFETY: the constant is a valid CFString.
            let k = unsafe { kVTEncodeFrameOptionKey_ForceKeyFrame };
            CFDictionary::<CFString, CFType>::from_slices(&[k], &[cf(CFBoolean::new(true))])
        });
        // SAFETY: a valid session and image buffer; the encoder retains the buffer.
        let status = unsafe {
            self.session.encode_frame(
                buf,
                pts,
                kCMTimeInvalid,
                props.as_ref().map(|p| p.as_opaque()),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            tracing::warn!(status, "encode failed");
        }
        status == 0
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: completes pending frames, then stops callbacks; after that the sink can go.
        unsafe {
            self.session.complete_frames(kCMTimeInvalid);
            self.session.invalidate();
            drop(Arc::from_raw(self.sink));
        }
    }
}

/// The encoder's output callback: one encoded frame (or an error).
unsafe extern "C-unwind" fn on_encoded(
    refcon: *mut c_void,
    _source: *mut c_void,
    status: i32,
    _flags: VTEncodeInfoFlags,
    sample: *mut CMSampleBuffer,
) {
    // SAFETY: `refcon` is the encoder's Sink, alive while the session is.
    let sink = unsafe { &*(refcon as *const Sink) };
    // SAFETY: a valid sample buffer for the duration of the callback, or null.
    let sample = if status == 0 {
        // SAFETY: on success, a valid sample buffer for the duration of the callback.
        unsafe { sample.as_ref() }
    } else {
        None
    };
    let Some(sample) = sample else {
        sink.dropped();
        return;
    };
    match unsafe { encoded(sample, sink) } {
        Some(e) => {
            if sink.out.send(e).is_err() {
                sink.dropped();
            }
        }
        None => sink.dropped(),
    }
}

/// Is this sample marked "not sync" (not a key frame)?
unsafe fn not_sync(sample: &CMSampleBuffer) -> bool {
    let Some(arr) = (unsafe { sample.sample_attachments_array(false) }) else {
        return false;
    };
    if arr.count() == 0 {
        return false;
    }
    let dict = unsafe { arr.value_at_index(0) } as *const CFDictionary;
    let Some(dict) = (unsafe { dict.as_ref() }) else {
        return false;
    };
    let key: &CFString = unsafe { kCMSampleAttachmentKey_NotSync };
    let v = unsafe { dict.value((key as *const CFString).cast()) } as *const CFType;
    match unsafe { v.as_ref() } {
        Some(v) => v.downcast_ref::<CFBoolean>().is_none_or(|b| b.as_bool()),
        None => false,
    }
}

/// The `avcC` record of a sample's format.
unsafe fn avcc(sample: &CMSampleBuffer) -> Option<Vec<u8>> {
    let fd = unsafe { sample.format_description() }?;
    let atoms =
        unsafe { fd.extension(kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms) }?;
    let dict = atoms.downcast_ref::<CFDictionary>()?;
    let key = CFString::from_static_str("avcC");
    let v = unsafe { dict.value((&*key as *const CFString).cast()) } as *const CFType;
    let data = unsafe { v.as_ref() }?.downcast_ref::<CFData>()?;
    Some(data.to_vec())
}

unsafe fn encoded(sample: &CMSampleBuffer, sink: &Sink) -> Option<Encoded> {
    let key = !unsafe { not_sync(sample) };
    let block = unsafe { sample.data_buffer() }?;
    let len = unsafe { block.data_length() };
    sink.last_size.store(len, Ordering::Release);
    let mut data = vec![0u8; len];
    let status =
        unsafe { block.copy_data_bytes(0, len, NonNull::new(data.as_mut_ptr().cast::<c_void>())?) };
    if status != 0 {
        return None;
    }
    let pts = unsafe { sample.presentation_time_stamp() };
    let ts_us = if pts.timescale > 0 {
        (pts.value as i128 * 1_000_000 / pts.timescale as i128) as u64
    } else {
        0
    };
    let mut config = None;
    if key && let Some(record) = unsafe { avcc(sample) } {
        let next = VideoConfig {
            codec: avc_codec(&record)?,
            description: record,
            width: sink.width,
            height: sink.height,
        };
        let mut sent = sink.sent_config.lock().unwrap_or_else(|e| e.into_inner());
        if sent.as_ref() != Some(&next) {
            *sent = Some(next.clone());
            config = Some(next);
        }
    }
    Some(Encoded {
        key,
        ts_us,
        data,
        config,
    })
}

// ---- capture → encoder, with flow control ---------------------------------------------

type Buf = SendBox<CFRetained<CVImageBuffer>>;

struct Pipeline {
    encoder: Mutex<Option<Encoder>>,
    /// The newest capture not yet encoded (the window was full).
    pending: Mutex<Option<Buf>>,
    /// The last capture encoded, to re-encode as a key frame on request.
    last: Mutex<Option<Buf>>,
    inflight: Arc<AtomicU32>,
    force_key: AtomicBool,
    started: Instant,
    /// When the last frame went to the encoder.
    encoded_at: Mutex<Instant>,
    last_size: Arc<AtomicUsize>,
    /// Re-encodes of the still picture since the screen last changed.
    refined: AtomicU32,
    /// A refining pass smaller than this adds little: the picture is as sharp as it gets.
    settled_bytes: usize,
}

/// Refining a still screen: macOS only sends a frame when something changes, so the last
/// frame of a movement (or a key frame, which the rate control keeps small) would stay soft
/// for as long as the screen is still. Re-encoding the same picture lets the encoder add back
/// detail pass by pass (at 4K: about 10, 50, 80, 100, 120, 150, 35, 15, 7 KB) until a
/// pass comes out small. Only when nothing is outstanding, so it never delays a real change.
const REFINE_AFTER: Duration = Duration::from_millis(50);
/// At most this many passes per still picture, whatever their size.
const REFINE_MAX: u32 = 16;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Pipeline {
    fn frame(&self, buf: Buf) {
        self.refined.store(0, Ordering::Release);
        if self.inflight.load(Ordering::Acquire) >= WINDOW {
            *lock(&self.pending) = Some(buf);
        } else {
            self.encode(buf);
        }
    }

    fn encode(&self, buf: Buf) {
        let enc = lock(&self.encoder);
        let Some(enc) = enc.as_ref() else { return };
        self.inflight.fetch_add(1, Ordering::AcqRel);
        let key = self.force_key.swap(false, Ordering::AcqRel);
        *lock(&self.encoded_at) = Instant::now();
        if !enc.encode(&buf.0, self.started.elapsed(), key) {
            let _ = self
                .inflight
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
            if key {
                self.force_key.store(true, Ordering::Release);
            }
        }
        *lock(&self.last) = Some(buf);
    }

    fn flush(&self) {
        if self.inflight.load(Ordering::Acquire) < WINDOW {
            let next = lock(&self.pending).take();
            if let Some(b) = next {
                self.encode(b);
            }
        }
    }

    fn ack(&self) {
        let _ = self
            .inflight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
        self.flush();
    }

    fn keyframe(&self) {
        self.force_key.store(true, Ordering::Release);
        self.refined.store(0, Ordering::Release);
        {
            let mut pending = lock(&self.pending);
            if pending.is_none() {
                *pending = lock(&self.last).as_ref().map(|b| SendBox(b.0.clone()));
            }
        }
        self.flush();
    }

    /// Called regularly: one more refining pass of a still screen, if it needs one.
    fn refine(&self) {
        if self.inflight.load(Ordering::Acquire) > 0
            || self.refined.load(Ordering::Acquire) >= REFINE_MAX
            || self.last_size.load(Ordering::Acquire) < self.settled_bytes
            || lock(&self.encoded_at).elapsed() < REFINE_AFTER
            || lock(&self.pending).is_some()
        {
            return;
        }
        let Some(buf) = lock(&self.last).as_ref().map(|b| SendBox(b.0.clone())) else {
            return;
        };
        self.refined.fetch_add(1, Ordering::AcqRel);
        self.encode(buf);
    }

    fn reset_window(&self) {
        self.inflight.store(0, Ordering::Release);
        self.flush();
    }
}

/// Is this a complete new frame (not an "idle, nothing changed" one)?
unsafe fn frame_complete(sample: &CMSampleBuffer) -> bool {
    let Some(arr) = (unsafe { sample.sample_attachments_array(false) }) else {
        return false;
    };
    if arr.count() == 0 {
        return false;
    }
    let dict = unsafe { arr.value_at_index(0) } as *const CFDictionary;
    let Some(dict) = (unsafe { dict.as_ref() }) else {
        return false;
    };
    let key = unsafe { SCStreamFrameInfoStatus };
    let v = unsafe { dict.value((key as *const _ as *const c_void).cast()) } as *const CFType;
    let status = unsafe { v.as_ref() }
        .and_then(|v| v.downcast_ref::<CFNumber>())
        .and_then(|n| n.as_i64());
    status == Some(SCFrameStatus::Complete.0 as i64)
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[name = "TermlabScreenOutput"]
    #[ivars = Arc<Pipeline>]
    struct Output;

    unsafe impl NSObjectProtocol for Output {}

    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            if kind != SCStreamOutputType::Screen {
                return;
            }
            // SAFETY: a valid sample buffer for this callback.
            unsafe {
                if !frame_complete(sample) {
                    return;
                }
                if let Some(buf) = sample.image_buffer() {
                    self.ivars().frame(SendBox(buf));
                }
            }
        }
    }
);

impl Output {
    fn new(pipe: Arc<Pipeline>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(pipe);
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

/// Where a display is on the global screen, in points: (x, y, width, height).
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One display being captured and encoded.
pub struct Stream {
    pipe: Arc<Pipeline>,
    stream: Retained<SCStream>,
    _output: Retained<Output>,
    _queue: DispatchRetained<DispatchQueue>,
    display: u32,
    bounds: Bounds,
}

// SAFETY: SCStream's methods may be called from any thread; the rest is Arc/thread-safe.
unsafe impl Send for Stream {}

fn wait_for(
    what: &str,
    run: impl FnOnce(&block2::DynBlock<dyn Fn(*mut NSError)>),
) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let block = RcBlock::new(move |err: *mut NSError| {
        let r = if err.is_null() {
            Ok(())
        } else {
            Err(error_text(err, "failed"))
        };
        let _ = tx.send(r);
    });
    run(&block);
    match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(anyhow!("{what}: {e}")),
        Err(_) => Err(anyhow!("{what}: no answer")),
    }
}

impl Stream {
    pub fn start(
        display: u32,
        quality: Quality,
        out: UnboundedSender<Encoded>,
    ) -> anyhow::Result<Self> {
        let content = shareable()?;
        // SAFETY: a getter on a valid SCShareableContent.
        let list = unsafe { content.displays() };
        let d = list
            .iter()
            // SAFETY: a getter on a valid SCDisplay.
            .find(|d| unsafe { d.displayID() } == display)
            .or_else(|| list.firstObject())
            .context("There is no display to share.")?;
        // SAFETY: getters on a valid SCDisplay.
        let (id, pw, ph) = unsafe {
            (
                d.displayID(),
                d.width().max(1) as f64,
                d.height().max(1) as f64,
            )
        };
        let (max_width, fps, bitrate) = quality.params();
        let native = (pw * display_scale(id)).round() as u32;
        let width = native.min(max_width) & !1;
        let height = ((width as f64 * ph / pw).round() as u32) & !1;

        let inflight = Arc::new(AtomicU32::new(0));
        let last_size = Arc::new(AtomicUsize::new(0));
        let sink = Arc::new(Sink {
            out,
            inflight: inflight.clone(),
            last_size: last_size.clone(),
            width,
            height,
            sent_config: Mutex::new(None),
        });
        let encoder = Encoder::new(width, height, fps, bitrate, sink)?;
        let pipe = Arc::new(Pipeline {
            encoder: Mutex::new(Some(encoder)),
            pending: Mutex::new(None),
            last: Mutex::new(None),
            inflight,
            force_key: AtomicBool::new(true),
            started: Instant::now(),
            encoded_at: Mutex::new(Instant::now()),
            last_size,
            refined: AtomicU32::new(0),
            // About 5 KB at 4K, 1.4 KB at 1080p.
            settled_bytes: (width as usize * height as usize / 1500).max(512),
        });

        // SAFETY: standard ScreenCaptureKit setup with valid objects.
        let (stream, output, queue) = unsafe {
            let filter = SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &d,
                &NSArray::new(),
            );
            let cfg = SCStreamConfiguration::new();
            cfg.setWidth(width as usize);
            cfg.setHeight(height as usize);
            cfg.setPixelFormat(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
            cfg.setMinimumFrameInterval(CMTime::new(1, fps as i32));
            cfg.setShowsCursor(true);
            cfg.setQueueDepth(5);
            cfg.setCapturesAudio(false);
            let stream = SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &cfg,
                None,
            );
            let output = Output::new(pipe.clone());
            let queue = DispatchQueue::new("dev.termlab.screen.capture", None);
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*output),
                    SCStreamOutputType::Screen,
                    Some(&queue),
                )
                .map_err(|e| anyhow!("Screen capture: {}", e.localizedDescription()))?;
            (stream, output, queue)
        };
        // SAFETY: a valid stream; the block outlives the call.
        wait_for("Screen capture couldn't start", |b| unsafe {
            stream.startCaptureWithCompletionHandler(Some(b))
        })?;
        let r = CGDisplayBounds(id);
        Ok(Stream {
            pipe,
            stream,
            _output: output,
            _queue: queue,
            display: id,
            bounds: Bounds {
                x: r.origin.x,
                y: r.origin.y,
                w: r.size.width,
                h: r.size.height,
            },
        })
    }

    pub fn display(&self) -> u32 {
        self.display
    }

    pub fn bounds(&self) -> Bounds {
        self.bounds
    }

    /// The browser decoded a frame.
    pub fn ack(&self) {
        self.pipe.ack();
    }

    /// Send a key frame next (re-encoding the current screen when nothing changes).
    pub fn keyframe(&self) {
        self.pipe.keyframe();
    }

    /// Sharpen a still screen (see [`REFINE_AFTER`]); call every [`super::REFINE_TICK`].
    pub fn refine(&self) {
        self.pipe.refine();
    }

    /// Acknowledgements stopped coming (a hiccup): open the window again.
    pub fn reset_window(&self) {
        self.pipe.reset_window();
    }

    pub fn inflight(&self) -> u32 {
        self.pipe.inflight.load(Ordering::Acquire)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: a valid stream; the block outlives the call.
        let _ = wait_for("stop", |b| unsafe {
            self.stream.stopCaptureWithCompletionHandler(Some(b))
        });
        lock(&self.pipe.encoder).take();
        lock(&self.pipe.pending).take();
        lock(&self.pipe.last).take();
    }
}
