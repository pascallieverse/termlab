//! The pointer on a Mac: its position from a fresh CGEvent, its picture from AppKit's
//! `NSCursor.currentSystemCursor` (whatever app shows it), at the best resolution it has.

use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGEvent, CGImage, CGImageAlphaInfo,
};
use objc2_foundation::NSArray;

use super::super::cursor::{Cursor, Shape, Shapes, unpremultiply};

// `NSCursor` lives in AppKit.
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

/// The picture is looked at less often than the position (it rarely changes, and reading it
/// costs a little drawing).
const SHAPE_EVERY: Duration = Duration::from_millis(48);

/// `kCGBitmapByteOrder32Big`: with premultiplied-last alpha, R G B A in memory.
const BYTE_ORDER_32_BIG: u32 = 4 << 12;

#[derive(Default)]
pub struct Reader {
    shapes: Shapes,
    shape: Option<Arc<Shape>>,
    shape_at: Option<Instant>,
}

impl Reader {
    pub fn read(&mut self) -> Cursor {
        let at = CGEvent::location(CGEvent::new(None).as_deref());
        if self.shape_at.is_none_or(|t| t.elapsed() >= SHAPE_EVERY) {
            self.shape_at = Some(Instant::now());
            if let Some(s) = autoreleasepool(|_| self.picture()) {
                self.shape = Some(s);
            }
        }
        Cursor {
            x: at.x,
            y: at.y,
            visible: true,
            shape: self.shape.clone(),
        }
    }

    fn picture(&mut self) -> Option<Arc<Shape>> {
        let class = AnyClass::get(c"NSCursor")?;
        // SAFETY: documented AppKit methods with these signatures; the objects returned are
        // retained here (or autoreleased into the pool around this call).
        unsafe {
            let cursor: Option<Retained<AnyObject>> = msg_send![class, currentSystemCursor];
            let cursor = cursor?;
            let image: Option<Retained<AnyObject>> = msg_send![&cursor, image];
            let image = image?;
            let hot: CGPoint = msg_send![&cursor, hotSpot];
            let size: CGSize = msg_send![&image, size];
            if size.width <= 0.0 || size.height <= 0.0 {
                return None;
            }
            // The 2× representation (pointers come at 1×, 2×, 5× and 10×): Retina sharp,
            // and small enough to send.
            let want = (size.width * 2.0).ceil() as isize;
            let reps: Option<Retained<NSArray<AnyObject>>> = msg_send![&image, representations];
            let mut best: Option<(isize, Retained<AnyObject>)> = None;
            for rep in reps?.iter() {
                let w: isize = msg_send![&rep, pixelsWide];
                let better = match &best {
                    None => true,
                    // Closest to 2× from above; otherwise the biggest below it.
                    Some((bw, _)) if *bw < want => w > *bw,
                    Some((bw, _)) => w >= want && w < *bw,
                };
                if better {
                    best = Some((w, rep));
                }
            }
            let none = std::ptr::null_mut::<AnyObject>();
            let rect = std::ptr::null_mut::<CGRect>();
            let cg: *mut CGImage = match &best {
                Some((w, rep)) if *w > 0 => {
                    msg_send![rep, CGImageForProposedRect: rect, context: none, hints: none]
                }
                _ => msg_send![&image, CGImageForProposedRect: rect, context: none, hints: none],
            };
            let cg = cg.as_ref()?;
            let (w, h) = (CGImage::width(Some(cg)), CGImage::height(Some(cg)));
            if w == 0 || h == 0 || w > 256 || h > 256 {
                return None;
            }
            let mut rgba = vec![0u8; w * h * 4];
            let space = CGColorSpace::new_device_rgb()?;
            let ctx = CGBitmapContextCreate(
                rgba.as_mut_ptr().cast(),
                w,
                h,
                8,
                w * 4,
                Some(&space),
                CGImageAlphaInfo::PremultipliedLast.0 | BYTE_ORDER_32_BIG,
            )?;
            CGContext::draw_image(
                Some(&ctx),
                CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64)),
                Some(cg),
            );
            drop(ctx);
            unpremultiply(&mut rgba);
            let scale = w as f64 / size.width;
            self.shapes.intern(
                w as u32,
                h as u32,
                (hot.x * scale).round().max(0.0) as u32,
                (hot.y * scale).round().max(0.0) as u32,
                scale,
                rgba,
            )
        }
    }
}
