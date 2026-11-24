//! Windows: displays from `EnumDisplayMonitors`; the picture from DXGI desktop duplication
//! ([`dxgi`]), or GDI (`BitBlt` of the desktop) where that can't run; encoded by the GPU
//! ([`mf`]) or in software ([`super::soft`]); input through `SendInput`. The pointer is read
//! apart ([`cursor`]) for the browser to draw, or drawn into the picture.
//!
//! The process is per-monitor DPI aware, so every coordinate here is in physical pixels.
//!
//! Limits (Windows' own rules for a program running as the signed-in user): no picture of
//! the sign-in screen, the lock screen or a UAC prompt (the secure desktop), and no input
//! into programs running as administrator.

#![allow(unsafe_code)]

mod cursor;
mod dxgi;
mod input;
mod mf;

use std::sync::Once;

use anyhow::{Context, bail};
use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GdiFlush, GetDC, GetMonitorInfoW,
    HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, ICONINFO,
};
use windows::core::BOOL;

pub use input::Input;

use super::soft::{Grab, Image, Opened, Stream};
use super::{Bounds, Permissions, Video};
use crate::proto::{DisplayInfo, Encoded, Encoding};

/// Software encoding: H.264 only.
pub const HEVC: bool = false;
/// No 4:4:4 either.
pub const HEVC444: bool = false;

/// Physical pixels everywhere (otherwise Windows scales coordinates for us, unevenly).
pub(crate) fn dpi_aware() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: a process-wide setting, made once before any window or DC use.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    });
}

/// No permission to ask for: a program running as the user may see and control its desktop.
pub fn permissions() -> Permissions {
    Permissions {
        screen: true,
        input: true,
    }
}

struct Monitor {
    rect: RECT,
    primary: bool,
    dpi: u32,
}

fn monitors() -> Vec<Monitor> {
    dpi_aware();
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the Vec passed below, alive for the whole enumeration.
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(m);
        BOOL(1)
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only touches `handles`, which outlives the call.
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(each),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        )
    };
    handles
        .into_iter()
        .filter_map(|h| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: a valid monitor handle and a properly sized struct.
            let ok = unsafe { GetMonitorInfoW(h, &mut info.monitorInfo as *mut MONITORINFO) };
            if !ok.as_bool() {
                return None;
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            // SAFETY: valid handle and out pointers.
            let _ = unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
            Some(Monitor {
                rect: info.monitorInfo.rcMonitor,
                primary: info.monitorInfo.dwFlags & 1 != 0,
                dpi: dx.max(48),
            })
        })
        .collect()
}

/// Display ids are 1-based positions in Windows' monitor order.
pub fn displays() -> anyhow::Result<Vec<DisplayInfo>> {
    let list = monitors();
    if list.is_empty() {
        bail!("There is no display to share.");
    }
    Ok(list
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let scale = f64::from(m.dpi) / 96.0;
            let (w, h) = (m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
            DisplayInfo {
                id: i as u32 + 1,
                name: format!("Display {}", i + 1),
                // Like the Mac: logical size; times `scale` gives the pixels.
                width: (f64::from(w) / scale).round() as u32,
                height: (f64::from(h) / scale).round() as u32,
                scale,
                primary: m.primary,
            }
        })
        .collect())
}

pub fn start(
    display: u32,
    enc: Encoding,
    out: UnboundedSender<Encoded>,
) -> anyhow::Result<Box<dyn Video>> {
    let draw_cursor = enc.draw_cursor;
    Ok(Box::new(Stream::start(enc, out, mf::encoder, move || {
        open(display, draw_cursor)
    })?))
}

fn open(display: u32, draw_cursor: bool) -> anyhow::Result<Opened> {
    let list = monitors();
    let (index, m) = list
        .iter()
        .enumerate()
        .find(|(i, _)| *i as u32 + 1 == display)
        .or_else(|| list.iter().enumerate().find(|(_, m)| m.primary))
        .or_else(|| list.iter().enumerate().next())
        .context("There is no display to share.")?;
    let r = m.rect;
    let (w, h) = ((r.right - r.left).max(2), (r.bottom - r.top).max(2));
    let scale = f64::from(m.dpi) / 96.0;
    // The GPU's copy of the desktop where it works, else GDI's.
    let grab: Box<dyn Grab> = match dxgi::Dxgi::new(r, scale, draw_cursor) {
        Ok(d) => {
            tracing::info!("capturing with DXGI desktop duplication");
            Box::new(d)
        }
        Err(e) => {
            tracing::info!(error = %e, "capturing with GDI");
            let mut g = Gdi::new(r.left, r.top, w, h)?;
            g.cursor = draw_cursor;
            Box::new(g)
        }
    };
    Ok(Opened {
        grab,
        pointer: Some(Box::new(cursor::WinPointer::new(scale))),
        display: index as u32 + 1,
        bounds: Bounds {
            x: f64::from(r.left),
            y: f64::from(r.top),
            w: f64::from(w),
            h: f64::from(h),
        },
        width: w as u32,
        height: h as u32,
    })
}

/// Copies one monitor's part of the desktop into a DIB section.
struct Gdi {
    screen: HDC,
    mem: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    /// Draw the pointer in (the browser doesn't draw it itself).
    cursor: bool,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Gdi {
    fn new(x: i32, y: i32, w: i32, h: i32) -> anyhow::Result<Self> {
        dpi_aware();
        // SAFETY: standard GDI setup; every handle is released in Drop (or below on failure).
        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                bail!("The screen can't be read.");
            }
            let mem = CreateCompatibleDC(Some(screen));
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    // Negative: top-down rows.
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap =
                match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                    Ok(b) if !bits.is_null() => b,
                    _ => {
                        let _ = DeleteDC(mem);
                        ReleaseDC(None, screen);
                        bail!("The screen can't be read.");
                    }
                };
            let old = SelectObject(mem, HGDIOBJ::from(bitmap));
            Ok(Gdi {
                screen,
                mem,
                bitmap,
                old,
                bits: bits.cast(),
                cursor: true,
                x,
                y,
                w,
                h,
            })
        }
    }

    /// The pointer, as Windows would draw it.
    fn draw_cursor(&self) {
        // SAFETY: valid DCs; the icon info bitmaps are deleted after use.
        unsafe {
            let mut ci = CURSORINFO {
                cbSize: std::mem::size_of::<CURSORINFO>() as u32,
                ..Default::default()
            };
            if GetCursorInfo(&mut ci).is_err() || ci.flags.0 & CURSOR_SHOWING.0 == 0 {
                return;
            }
            let mut ii = ICONINFO::default();
            let hot = if GetIconInfo(ci.hCursor.into(), &mut ii).is_ok() {
                if !ii.hbmMask.is_invalid() {
                    let _ = DeleteObject(ii.hbmMask.into());
                }
                if !ii.hbmColor.is_invalid() {
                    let _ = DeleteObject(ii.hbmColor.into());
                }
                POINT {
                    x: ii.xHotspot as i32,
                    y: ii.yHotspot as i32,
                }
            } else {
                POINT::default()
            };
            let _ = DrawIconEx(
                self.mem,
                ci.ptScreenPos.x - hot.x - self.x,
                ci.ptScreenPos.y - hot.y - self.y,
                ci.hCursor.into(),
                0,
                0,
                0,
                None,
                DI_NORMAL,
            );
        }
    }
}

impl Grab for Gdi {
    fn grab(&mut self) -> anyhow::Result<Option<Image>> {
        // SAFETY: valid DCs of the right size; `bits` is the DIB section's memory, read
        // after GdiFlush.
        unsafe {
            if BitBlt(
                self.mem,
                0,
                0,
                self.w,
                self.h,
                Some(self.screen),
                self.x,
                self.y,
                SRCCOPY,
            )
            .is_err()
            {
                // The secure desktop (lock screen, UAC) or a mode change: try again later.
                return Ok(None);
            }
            if self.cursor {
                self.draw_cursor();
            }
            let _ = GdiFlush();
            let len = self.w as usize * self.h as usize * 4;
            let data = std::slice::from_raw_parts(self.bits, len).to_vec();
            Ok(Some(Image {
                data,
                width: self.w as u32,
                height: self.h as u32,
                stride: self.w as usize * 4,
            }))
        }
    }
}

impl Drop for Gdi {
    fn drop(&mut self) {
        // SAFETY: the handles made in `new`, released once.
        unsafe {
            SelectObject(self.mem, self.old);
            let _ = DeleteObject(HGDIOBJ::from(self.bitmap));
            let _ = DeleteDC(self.mem);
            ReleaseDC(None, self.screen);
        }
    }
}
