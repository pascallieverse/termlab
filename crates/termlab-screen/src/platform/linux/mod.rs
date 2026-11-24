//! Linux: X11 sessions are captured and controlled directly ([`x11`]); Wayland sessions go
//! through the desktop's remote desktop portal ([`wayland`]). Both encode with
//! [`super::soft`].

#![allow(unsafe_code)]

mod hardware;
mod wayland;
mod x11;

use std::path::Path;

use anyhow::bail;
use tokio::sync::mpsc::UnboundedSender;

use super::soft::Stream;
use super::{Permissions, Video};
use crate::proto::{DisplayInfo, Encoded, Encoding, InputEvent};

/// Software encoding: H.264 only.
pub const HEVC: bool = false;
/// No 4:4:4 either.
pub const HEVC444: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    Wayland,
    X11,
    None,
}

fn session() -> Session {
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
    {
        Session::Wayland
    } else if std::env::var_os("DISPLAY").is_some() {
        Session::X11
    } else {
        Session::None
    }
}

const NO_SESSION: &str =
    "There's no graphical session to share (Termlab runs without a display here).";

/// Where the Wayland portal's approval is kept.
pub fn set_data_dir(dir: &Path) {
    wayland::set_data_dir(dir);
}

/// `termlab-screen authorize <data-dir>`: Wayland's one-time approval.
pub fn authorize(dir: &Path) -> anyhow::Result<()> {
    match session() {
        Session::Wayland => wayland::authorize(dir),
        Session::X11 => Ok(()),
        Session::None => bail!(NO_SESSION),
    }
}

/// X11: nothing to ask. Wayland: approved once on the computer (until then the first
/// session needs someone there to approve it).
pub fn permissions() -> Permissions {
    match session() {
        Session::X11 => Permissions {
            screen: true,
            input: true,
        },
        Session::Wayland => {
            let ok = wayland::approved();
            Permissions {
                screen: ok,
                input: ok,
            }
        }
        Session::None => Permissions {
            screen: false,
            input: false,
        },
    }
}

pub fn displays() -> anyhow::Result<Vec<DisplayInfo>> {
    match session() {
        Session::X11 => x11::displays(),
        Session::Wayland => wayland::displays(),
        Session::None => bail!(NO_SESSION),
    }
}

pub fn start(
    display: u32,
    enc: Encoding,
    out: UnboundedSender<Encoded>,
) -> anyhow::Result<Box<dyn Video>> {
    let kind = session();
    let draw_cursor = enc.draw_cursor;
    Ok(Box::new(Stream::start(enc, out, hardware::encoder, move || match kind {
        Session::X11 => x11::open(display, draw_cursor),
        Session::Wayland => wayland::open(display),
        Session::None => bail!(NO_SESSION),
    })?))
}

pub enum Input {
    X11(Box<x11::Input>),
    Wayland(wayland::Input),
    None,
}

impl Input {
    pub fn new() -> Self {
        match session() {
            Session::X11 => match x11::Input::new() {
                Ok(i) => Input::X11(Box::new(i)),
                Err(e) => {
                    tracing::warn!(error = %e, "no X11 input");
                    Input::None
                }
            },
            Session::Wayland => Input::Wayland(wayland::Input::new()),
            Session::None => Input::None,
        }
    }

    pub fn apply(&mut self, ev: InputEvent) {
        match self {
            Input::X11(i) => i.apply(ev),
            Input::Wayland(i) => i.apply(ev),
            Input::None => {}
        }
    }
}
