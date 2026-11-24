//! Screen capture, encoding and input, per platform.
//!
//! Each platform provides:
//! - `permissions()`: whether this process may capture the screen and send input;
//! - `displays()`: the displays that can be shared;
//! - `start(display, encoding, frames)`: captures and encodes one display ([`Video`]). At
//!   most [`WINDOW`] frames are unacknowledged; newer captures replace the one waiting, so a
//!   slow browser gets fewer frames instead of a growing delay. Dropping it stops the capture;
//! - `Input`: posts mouse and keyboard events;
//! - `HEVC`: whether it can encode HEVC; `HEVC444`: also with full colour detail (4:4:4).
//!
//! macOS uses ScreenCaptureKit and VideoToolbox. Windows and Linux grab the screen themselves
//! and share the software encoder in [`soft`]: Windows with GDI and `SendInput`; Linux with
//! X11 (shared memory, XTEST) or, under Wayland, the desktop's remote desktop portal.

use std::time::Duration;

use serde::Serialize;

/// Frames sent but not yet decoded by the browser.
pub const WINDOW: u32 = 6;

/// How often the session asks the stream to sharpen a still screen (`Stream::refine`).
pub const REFINE_TICK: Duration = Duration::from_millis(25);

/// Refining a still screen: the operating system only hands over a new picture when something
/// changes, so the last frame of a movement (or a key frame, which the rate control keeps
/// small) would stay soft for as long as the screen is still. Re-encoding the same picture
/// lets the encoder add back detail pass by pass (at 4K on a Mac: about 10, 50, 80, 100, 120,
/// 150, 35, 15, 7 KB) until a pass comes out small. Only when nothing is outstanding, so it
/// never delays a real change.
pub const REFINE_AFTER: Duration = Duration::from_millis(50);
/// At most this many passes per still picture, whatever their size.
pub const REFINE_MAX: u32 = 16;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Permissions {
    /// Screen Recording (macOS) or equivalent.
    pub screen: bool,
    /// Accessibility (macOS): posting mouse and keyboard events.
    pub input: bool,
}

/// Where a display is on the global screen, in the units input events use there (points
/// on macOS, pixels on Windows and X11; the shared stream's pixels under Wayland).
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A display being captured and encoded.
pub trait Video: Send {
    fn display(&self) -> u32;
    fn bounds(&self) -> Bounds;
    /// The browser decoded a frame.
    fn ack(&self);
    /// Send a key frame next (re-encoding the current screen when nothing changes).
    fn keyframe(&self);
    /// Sharpen a still screen (see [`REFINE_AFTER`]); called every [`REFINE_TICK`].
    fn refine(&self);
    /// Acknowledgements stopped coming (a hiccup): open the window again.
    fn reset_window(&self);
    fn inflight(&self) -> u32;
    /// The capture stopped for good (why), so the session should end.
    fn failed(&self) -> Option<String> {
        None
    }
    /// The pointer now, where the platform can tell it apart from the picture (then the
    /// browser draws it; see [`cursor`]). Called about 60 times a second.
    fn cursor(&self) -> Option<cursor::Cursor> {
        None
    }
}

pub mod cursor;

#[cfg(any(not(target_os = "macos"), test))]
pub mod pc_keys;
#[cfg(any(not(target_os = "macos"), feature = "soft"))]
pub mod soft;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{HEVC, HEVC444, Input, displays, permissions, start};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{HEVC, HEVC444, Input, displays, permissions, start};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{HEVC, HEVC444, Input, authorize, displays, permissions, set_data_dir, start};

/// Elsewhere nothing needs approving ahead of time (Linux under Wayland does).
#[cfg(not(target_os = "linux"))]
pub fn authorize(_data_dir: &std::path::Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn set_data_dir(_data_dir: &std::path::Path) {}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod unsupported;
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub use unsupported::{HEVC, HEVC444, Input, displays, permissions, start};
