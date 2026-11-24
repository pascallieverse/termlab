//! Hardware H.264 on Windows: the GPU's Media Foundation encoder (NVIDIA NVENC, Intel Quick
//! Sync, AMD AMF all provide one), fed NV12 pictures from memory.
//!
//! Hardware encoders are asynchronous: they say when they want a picture
//! (`METransformNeedInput`) and when a frame is ready (`METransformHaveOutput`). With low
//! latency on and no B-frames, each picture comes out before the next goes in.

use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode,
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize,
    CODECAPI_AVEncVideoForceKeyFrame, CODECAPI_AVEncVideoMaxQP, CODECAPI_AVLowLatencyMode,
    ICodecAPI, IMFActivate, IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFTransform,
    METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_EVENT_FLAG_NO_WAIT,
    MF_LOW_LATENCY, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_YUV_MATRIX, MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK,
    MF_VERSION, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
    MFNominalRange_16_235, MFSTARTUP_NOSOCKET, MFSampleExtension_CleanPoint, MFStartup,
    MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_END_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
    MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoFormat_H264,
    MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFVideoTransferMatrix_BT709,
    eAVEncCommonRateControlMode_LowDelayVBR, eAVEncH264VProfile_High,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{Interface, PWSTR};

use super::super::soft::{Bits, FrameEncoder, Spec, Yuv, has_idr, write_nv12};

/// The platform hook: the first hardware encoder that takes these settings.
pub fn encoder(spec: &Spec) -> Option<Box<dyn FrameEncoder>> {
    match Mft::open(spec) {
        Ok(e) => Some(Box::new(e)),
        Err(e) => {
            tracing::info!(error = %e, "no hardware encoder: software");
            None
        }
    }
}

/// How long one frame may take before the encoder counts as stuck.
const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

struct Mft {
    transform: IMFTransform,
    events: Option<IMFMediaEventGenerator>,
    codec: Option<ICodecAPI>,
    /// Pictures the encoder asked for and hasn't had yet.
    wants: u32,
    /// The encoder hands out its own output samples.
    provides: bool,
    out_size: u32,
    fps: u32,
    frame: Vec<u8>,
    name: String,
}

// The encoder is made and used on the encode thread only.

fn set_u32(t: &IMFMediaType, key: &windows::core::GUID, v: u32) -> windows::core::Result<()> {
    // SAFETY: a valid media type and attribute key.
    unsafe { t.SetUINT32(key, v) }
}

fn set_pair(t: &IMFMediaType, key: &windows::core::GUID, hi: u32, lo: u32) -> windows::core::Result<()> {
    // SAFETY: as above; sizes and rates are packed as (hi << 32) | lo.
    unsafe { t.SetUINT64(key, (u64::from(hi) << 32) | u64::from(lo)) }
}

fn friendly_name(a: &IMFActivate) -> String {
    let mut p = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: GetAllocatedString hands out a CoTaskMem string, freed here.
    unsafe {
        if a
            .GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut p, &mut len)
            .is_ok()
        {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as *const _));
            s
        } else {
            "hardware encoder".into()
        }
    }
}

impl Mft {
    fn open(spec: &Spec) -> anyhow::Result<Self> {
        // SAFETY: per-thread COM and process-wide Media Foundation set-up (both may be called
        // more than once; never shut down, as other streams may still use them).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).context("Media Foundation")?;
        }
        let input = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: valid type infos; `list` receives a CoTaskMem array of `count` activates.
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
                Some(&input),
                Some(&output),
                &mut list,
                &mut count,
            )
            .context("listing encoders")?;
        }
        if list.is_null() || count == 0 {
            bail!("no hardware H.264 encoder");
        }
        // SAFETY: `count` entries, each taken (and so released) once; then the array is freed.
        let activates: Vec<IMFActivate> = unsafe {
            let v = (0..count as usize)
                .filter_map(|i| (*list.add(i)).take())
                .collect();
            CoTaskMemFree(Some(list as *const _));
            v
        };
        let mut last = anyhow!("no hardware H.264 encoder");
        for a in &activates {
            let name = friendly_name(a);
            match Mft::setup(a, spec, &name) {
                Ok(m) => {
                    tracing::info!(encoder = %name, "hardware encoder");
                    return Ok(m);
                }
                Err(e) => {
                    tracing::info!(encoder = %name, error = %e, "hardware encoder refused");
                    // SAFETY: frees what ActivateObject made.
                    let _ = unsafe { a.ShutdownObject() };
                    last = e;
                }
            }
        }
        Err(last)
    }

    fn setup(a: &IMFActivate, spec: &Spec, name: &str) -> anyhow::Result<Self> {
        let (w, h) = (spec.width as u32, spec.height as u32);
        let fps = spec.enc.fps.max(1);
        // SAFETY: standard Media Foundation encoder set-up on valid objects.
        unsafe {
            let transform: IMFTransform = a.ActivateObject()?;
            let attrs = transform.GetAttributes()?;
            let is_async = attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) != 0;
            if is_async {
                attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
            }
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);

            let codec = transform.cast::<ICodecAPI>().ok();
            if let Some(c) = &codec {
                let set = |key: &windows::core::GUID, v: u32| {
                    let _ = c.SetValue(key, &VARIANT::from(v));
                };
                set(
                    &CODECAPI_AVEncCommonRateControlMode,
                    eAVEncCommonRateControlMode_LowDelayVBR.0 as u32,
                );
                set(&CODECAPI_AVEncCommonMeanBitRate, spec.enc.bitrate);
                set(&CODECAPI_AVEncMPVDefaultBPictureCount, 0);
                // Key frames when asked for, not on a timer (0 means "the encoder's
                // default" to some, so a long interval instead: ten minutes).
                set(&CODECAPI_AVEncMPVGOPSize, spec.enc.fps.max(1) * 600);
                set(&CODECAPI_AVEncVideoMaxQP, u32::from(spec.max_qp));
                let _ = c.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            }

            // Output first (encoders want to know what they make), then input.
            let out = MFCreateMediaType()?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            set_u32(&out, &MF_MT_AVG_BITRATE, spec.enc.bitrate)?;
            set_pair(&out, &MF_MT_FRAME_SIZE, w, h)?;
            set_pair(&out, &MF_MT_FRAME_RATE, fps, 1)?;
            set_pair(&out, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
            set_u32(&out, &MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            set_u32(&out, &MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            let _ = set_u32(&out, &MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32);
            let _ = set_u32(&out, &MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32);
            transform.SetOutputType(0, &out, 0).context("output type")?;

            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            set_pair(&input, &MF_MT_FRAME_SIZE, w, h)?;
            set_pair(&input, &MF_MT_FRAME_RATE, fps, 1)?;
            set_pair(&input, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
            set_u32(&input, &MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            transform.SetInputType(0, &input, 0).context("input type")?;

            let info = transform.GetOutputStreamInfo(0)?;
            let provides = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0)
                    as u32
                != 0;
            transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            let events = if is_async {
                Some(transform.cast::<IMFMediaEventGenerator>()?)
            } else {
                None
            };
            Ok(Mft {
                transform,
                events,
                codec,
                wants: 0,
                provides,
                out_size: info.cbSize.max(w * h),
                fps,
                frame: vec![0u8; (w * h * 3 / 2) as usize],
                name: format!("{name} (hardware)"),
            })
        }
    }

    /// The next event of an asynchronous encoder: `Some(true)` it wants a picture,
    /// `Some(false)` a frame is ready, `None` nothing yet.
    fn next_event(&self) -> anyhow::Result<Option<bool>> {
        let Some(events) = &self.events else {
            return Ok(None);
        };
        // SAFETY: a valid event generator; non-blocking.
        match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
            Ok(ev) => {
                // SAFETY: a valid event.
                let kind = unsafe { ev.GetType()? };
                if kind == METransformNeedInput.0 as u32 {
                    Ok(Some(true))
                } else if kind == METransformHaveOutput.0 as u32 {
                    Ok(Some(false))
                } else {
                    Ok(None)
                }
            }
            Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn sample(&mut self, yuv: &Yuv, at: Duration) -> anyhow::Result<IMFSample> {
        write_nv12(yuv, &mut self.frame);
        let len = self.frame.len() as u32;
        // SAFETY: a fresh buffer of `len` bytes, locked while copying into it.
        unsafe {
            let buffer = MFCreateMemoryBuffer(len)?;
            let mut p: *mut u8 = std::ptr::null_mut();
            buffer.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(self.frame.as_ptr(), p, self.frame.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(len)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            // In 100 ns units.
            sample.SetSampleTime((at.as_nanos() / 100) as i64)?;
            sample.SetSampleDuration(10_000_000 / i64::from(self.fps))?;
            Ok(sample)
        }
    }

    /// One frame out, if there is one.
    fn take_output(&mut self) -> anyhow::Result<Option<Bits>> {
        loop {
            // SAFETY: an output buffer as the encoder wants it (its own sample, or ours of
            // the size it asked for); the sample and events it returns are released here.
            let result = unsafe {
                let ours = if self.provides {
                    None
                } else {
                    let s = MFCreateSample()?;
                    s.AddBuffer(&MFCreateMemoryBuffer(self.out_size)?)?;
                    Some(s)
                };
                let mut buf = [MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: std::mem::ManuallyDrop::new(ours),
                    dwStatus: 0,
                    pEvents: std::mem::ManuallyDrop::new(None),
                }];
                let mut status = 0u32;
                let r = self.transform.ProcessOutput(0, &mut buf, &mut status);
                let sample = std::mem::ManuallyDrop::take(&mut buf[0].pSample);
                drop(std::mem::ManuallyDrop::take(&mut buf[0].pEvents));
                r.map(|()| sample)
            };
            match result {
                Ok(Some(sample)) => return self.read(&sample).map(Some),
                Ok(None) => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // The encoder settled its output format: take it and ask again.
                    // SAFETY: valid transform; the type it offers is set back as is.
                    unsafe {
                        let t = self.transform.GetOutputAvailableType(0, 0)?;
                        self.transform.SetOutputType(0, &t, 0)?;
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn read(&self, sample: &IMFSample) -> anyhow::Result<Bits> {
        // SAFETY: the buffer is locked while copied out.
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut p: *mut u8 = std::ptr::null_mut();
            let mut len = 0u32;
            buffer.Lock(&mut p, None, Some(&mut len))?;
            let data = std::slice::from_raw_parts(p, len as usize).to_vec();
            buffer.Unlock()?;
            let clean = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
            let key = clean || has_idr(&data);
            Ok(Bits { data, key })
        }
    }
}

impl FrameEncoder for Mft {
    fn encode(&mut self, yuv: &Yuv, key: bool, at: Duration) -> anyhow::Result<Option<Bits>> {
        let sample = self.sample(yuv, at)?;
        let end = Instant::now() + FRAME_TIMEOUT;
        // Asynchronous: wait until it wants a picture (frames still coming out are kept).
        let mut early: Option<Bits> = None;
        if self.events.is_some() {
            while self.wants == 0 {
                match self.next_event()? {
                    Some(true) => self.wants += 1,
                    Some(false) => {
                        if let Some(b) = self.take_output()? {
                            early = Some(b);
                        }
                    }
                    None if Instant::now() < end => std::thread::sleep(Duration::from_millis(1)),
                    None => bail!("the encoder stopped asking for pictures"),
                }
            }
        }
        if key && let Some(c) = &self.codec {
            // SAFETY: a valid codec API; applies to the next picture.
            let _ = unsafe { c.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1u32)) };
        }
        // SAFETY: a valid sample in the input type set up above.
        unsafe { self.transform.ProcessInput(0, &sample, 0)? };
        self.wants = self.wants.saturating_sub(1);
        if let Some(b) = early {
            return Ok(Some(b));
        }
        if self.events.is_none() {
            return self.take_output();
        }
        // Then wait for its frame.
        loop {
            match self.next_event()? {
                Some(true) => self.wants += 1,
                Some(false) => {
                    if let Some(b) = self.take_output()? {
                        return Ok(Some(b));
                    }
                }
                None if Instant::now() < end => std::thread::sleep(Duration::from_millis(1)),
                None => bail!("the encoder took too long"),
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for Mft {
    fn drop(&mut self) {
        // SAFETY: a valid transform, told the stream ends.
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }
    }
}
