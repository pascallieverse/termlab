//! Screen capture, encoding and input, per platform.
//!
//! Each platform provides:
//! - `permissions()`: whether this process may capture the screen and send input;
//! - `displays()`: the displays that can be shared;
//! - `Stream::start(display, quality, frames)`: captures and encodes one display. At most
//!   [`WINDOW`] frames are unacknowledged; newer captures replace the one waiting, so a slow
//!   browser gets fewer frames instead of a growing delay. `ack`, `keyframe`, `bounds`;
//!   dropping it stops the capture;
//! - `Input`: posts mouse and keyboard events.

use serde::Serialize;

/// Frames sent but not yet decoded by the browser.
pub const WINDOW: u32 = 4;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Permissions {
    /// Screen Recording (macOS) or equivalent.
    pub screen: bool,
    /// Accessibility (macOS): posting mouse and keyboard events.
    pub input: bool,
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Input, Stream, displays, permissions};

#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
pub use unsupported::{Input, Stream, displays, permissions};
