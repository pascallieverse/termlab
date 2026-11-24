//! The pointer on Windows: `GetCursorInfo` for where and which, its bitmaps for the picture
//! (read once per cursor handle).

use std::sync::Arc;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, GetCursorInfo, GetIconInfo, HCURSOR, ICONINFO,
};

use super::super::cursor::{Cursor, Shape, Shapes};
use super::super::soft::Pointer;

pub struct WinPointer {
    /// Physical pixels per display point on the shared monitor.
    scale: f64,
    shapes: Shapes,
    last: Option<(isize, Arc<Shape>)>,
}

impl WinPointer {
    pub fn new(scale: f64) -> Self {
        WinPointer {
            scale,
            shapes: Shapes::default(),
            last: None,
        }
    }
}

impl Pointer for WinPointer {
    fn read(&mut self) -> Option<Cursor> {
        let mut ci = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: a properly sized struct.
        unsafe { GetCursorInfo(&mut ci) }.ok()?;
        let visible = ci.flags.0 & CURSOR_SHOWING.0 != 0 && !ci.hCursor.is_invalid();
        let handle = ci.hCursor.0 as isize;
        if visible
            && self.last.as_ref().map(|l| l.0) != Some(handle)
            && let Some((w, h, hx, hy, rgba)) = picture(ci.hCursor)
            && let Some(shape) = self.shapes.intern(w, h, hx, hy, self.scale, rgba)
        {
            self.last = Some((handle, shape));
        }
        Some(Cursor {
            x: f64::from(ci.ptScreenPos.x),
            y: f64::from(ci.ptScreenPos.y),
            visible,
            shape: self.last.as_ref().map(|l| l.1.clone()),
        })
    }
}

/// A bitmap's pixels as 32-bit BGRA, top row first.
fn pixels(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    // SAFETY: a valid bitmap handle; GetDIBits writes at most w*h*4 bytes into `out`.
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(
            HGDIOBJ::from(bitmap),
            std::mem::size_of::<BITMAP>() as i32,
            Some((&mut bm as *mut BITMAP).cast()),
        ) == 0
        {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 || w > 512 || h > 1024 {
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut out = vec![0u8; (w * h * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            h as u32,
            Some(out.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        (lines == h).then_some((w as u32, h as u32, out))
    }
}

/// A cursor's picture as straight RGBA: (width, height, hot x, hot y, pixels).
fn picture(cursor: HCURSOR) -> Option<(u32, u32, u32, u32, Vec<u8>)> {
    let mut ii = ICONINFO::default();
    // SAFETY: a valid cursor handle; the bitmaps it hands out are deleted below.
    unsafe { GetIconInfo(cursor.into(), &mut ii) }.ok()?;
    let mask = (!ii.hbmMask.is_invalid()).then(|| pixels(ii.hbmMask)).flatten();
    let color = (!ii.hbmColor.is_invalid()).then(|| pixels(ii.hbmColor)).flatten();
    // SAFETY: GetIconInfo's copies, ours to delete.
    unsafe {
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(ii.hbmMask.into());
        }
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(ii.hbmColor.into());
        }
    }
    let (hx, hy) = (ii.xHotspot, ii.yHotspot);
    match (color, mask) {
        (Some((w, h, bgra)), mask) => {
            let alpha = bgra.chunks_exact(4).any(|p| p[3] != 0);
            let mut rgba = Vec::with_capacity(bgra.len());
            for (i, p) in bgra.chunks_exact(4).enumerate() {
                // Without an alpha channel, the mask says what's see-through (white).
                let a = if alpha {
                    p[3]
                } else {
                    match &mask {
                        Some((_, _, m)) if m.get(i * 4).is_some_and(|&v| v != 0) => 0,
                        _ => 255,
                    }
                };
                rgba.extend_from_slice(&[p[2], p[1], p[0], a]);
            }
            Some((w, h, hx, hy, rgba))
        }
        // Black and white: the mask holds AND (top half) and XOR (bottom half).
        (None, Some((w, h2, m))) => {
            let h = h2 / 2;
            Some((w, h, hx, hy, monochrome(w, h, &m)))
        }
        (None, None) => None,
    }
}

/// AND/XOR masks (32-bit, white = 1) to RGBA. "Invert the screen" pixels (the text beam) can't
/// be shown by a browser: they become black with a white edge, visible on any background.
fn monochrome(w: u32, h: u32, m: &[u8]) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let bit = |plane: usize, x: usize, y: usize| m[((plane * h + y) * w + x) * 4] != 0;
    let mut rgba = vec![0u8; w * h * 4];
    let mut invert = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let px = &mut rgba[(y * w + x) * 4..][..4];
            match (bit(0, x, y), bit(1, x, y)) {
                (false, false) => px.copy_from_slice(&[0, 0, 0, 255]),
                (false, true) => px.copy_from_slice(&[255, 255, 255, 255]),
                (true, false) => {}
                (true, true) => {
                    px.copy_from_slice(&[0, 0, 0, 255]);
                    invert[y * w + x] = true;
                }
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] != 0 {
                continue;
            }
            let near = (y.saturating_sub(1)..(y + 2).min(h))
                .any(|ny| (x.saturating_sub(1)..(x + 2).min(w)).any(|nx| invert[ny * w + nx]));
            if near {
                rgba[(y * w + x) * 4..][..4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverting_pixels_get_an_edge() {
        // 3×1: transparent, invert, transparent.
        let px = |white: bool| if white { [255u8; 4] } else { [0u8; 4] };
        let and = [px(true), px(true), px(true)];
        let xor = [px(false), px(true), px(false)];
        let m: Vec<u8> = and.iter().chain(xor.iter()).flatten().copied().collect();
        let out = monochrome(3, 1, &m);
        assert_eq!(&out[4..8], &[0, 0, 0, 255]);
        assert_eq!(&out[0..4], &[255, 255, 255, 255]);
        assert_eq!(&out[8..12], &[255, 255, 255, 255]);
    }
}
