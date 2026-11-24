//! The mouse pointer, sent apart from the video: its picture once per shape, its position as
//! it moves. The browser then draws it itself, so it moves without the round trip a pointer
//! drawn into the video has (and takes the right shape: text beam, hand, resize arrows).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// One pointer picture.
#[derive(Debug)]
pub struct Shape {
    /// Numbered per helper process; the browser keeps the ones it was sent.
    pub id: u32,
    pub width: u32,
    pub height: u32,
    /// The point that clicks, in image pixels.
    pub hot_x: u32,
    pub hot_y: u32,
    /// Image pixels per display point (2 for a Retina pointer on a Mac).
    pub scale: f64,
    /// Straight (not premultiplied) RGBA, row by row.
    pub rgba: Vec<u8>,
}

/// Where the pointer is.
#[derive(Debug, Clone)]
pub struct Cursor {
    /// On the global screen, in the units of [`super::Bounds`].
    pub x: f64,
    pub y: f64,
    /// Hidden (while typing, in a full-screen video, …).
    pub visible: bool,
    pub shape: Option<Arc<Shape>>,
}

/// Largest picture passed on (bigger ones are odd; the browser can't show them anyway).
const MAX_SIDE: u32 = 256;

/// Gives each distinct picture one id, so a shape seen again isn't sent again.
#[derive(Default)]
pub struct Shapes {
    known: Vec<(u64, Arc<Shape>)>,
}

impl Shapes {
    /// The shape for this picture (straight RGBA), numbered when new.
    pub fn intern(
        &mut self,
        width: u32,
        height: u32,
        hot_x: u32,
        hot_y: u32,
        scale: f64,
        rgba: Vec<u8>,
    ) -> Option<Arc<Shape>> {
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || rgba.len() != (width * height * 4) as usize
        {
            return None;
        }
        let mut h = DefaultHasher::new();
        (width, height, hot_x, hot_y, scale.to_bits()).hash(&mut h);
        rgba.hash(&mut h);
        let key = h.finish();
        if let Some((_, s)) = self.known.iter().find(|(k, _)| *k == key) {
            return Some(s.clone());
        }
        let shape = Arc::new(Shape {
            id: self.known.len() as u32 + 1,
            width,
            height,
            hot_x: hot_x.min(width - 1),
            hot_y: hot_y.min(height - 1),
            scale: if scale > 0.0 { scale } else { 1.0 },
            rgba,
        });
        // Apps with animated pointers make many; keep the list bounded (ids stay unique).
        if self.known.len() >= 512 {
            self.known.clear();
        }
        self.known.push((key, shape.clone()));
        Some(shape)
    }
}

/// Premultiplied → straight alpha, in place (RGBA or BGRA alike).
#[cfg_attr(windows, allow(dead_code))]
pub fn unpremultiply(px: &mut [u8]) {
    for p in px.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

/// Draws `shape` (straight RGBA) over a tightly packed BGRA picture, its top-left corner at
/// (`left`, `top`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn blend(data: &mut [u8], width: usize, height: usize, shape: &Shape, left: i32, top: i32) {
    let (sw, sh) = (shape.width as i32, shape.height as i32);
    for sy in 0..sh {
        let y = top + sy;
        if y < 0 || y >= height as i32 {
            continue;
        }
        for sx in 0..sw {
            let x = left + sx;
            if x < 0 || x >= width as i32 {
                continue;
            }
            let s = &shape.rgba[((sy * sw + sx) * 4) as usize..][..4];
            let a = u32::from(s[3]);
            if a == 0 {
                continue;
            }
            let d = &mut data[(y as usize * width + x as usize) * 4..][..3];
            for (di, si) in [(0, 2), (1, 1), (2, 0)] {
                d[di] = ((u32::from(s[si]) * a + u32::from(d[di]) * (255 - a) + 127) / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_picture_keeps_its_id() {
        let mut s = Shapes::default();
        let a = s.intern(2, 2, 0, 0, 1.0, vec![255; 16]).unwrap();
        let b = s.intern(2, 2, 1, 1, 1.0, vec![255; 16]).unwrap();
        let a2 = s.intern(2, 2, 0, 0, 1.0, vec![255; 16]).unwrap();
        assert_eq!(a.id, a2.id);
        assert_ne!(a.id, b.id);
        assert!(s.intern(2, 2, 0, 0, 1.0, vec![0; 3]).is_none());
    }

    #[test]
    fn unpremultiplies() {
        let mut px = [64, 32, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0];
        unpremultiply(&mut px);
        assert_eq!(px, [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn blends_over_bgra_and_clips() {
        let shape = Shape {
            id: 1,
            width: 2,
            height: 1,
            hot_x: 0,
            hot_y: 0,
            scale: 1.0,
            // Opaque red, then half-transparent white.
            rgba: vec![255, 0, 0, 255, 255, 255, 255, 128],
        };
        let mut data = vec![0u8; 2 * 2 * 4];
        blend(&mut data, 2, 2, &shape, 0, 1);
        assert_eq!(&data[8..11], &[0, 0, 255]);
        assert_eq!(&data[12..15], &[128, 128, 128]);
        // Off the edge: nothing breaks.
        blend(&mut data, 2, 2, &shape, -5, 7);
        blend(&mut data, 2, 2, &shape, 1, 0);
        assert_eq!(&data[4..7], &[0, 0, 255]);
    }
}
