//! Platforms without screen sharing: everything reports "not supported".

use anyhow::bail;
use tokio::sync::mpsc::UnboundedSender;

use super::{Permissions, Video};
use crate::proto::{DisplayInfo, Encoded, Encoding, InputEvent};

const NOT_YET: &str = "Screen sharing isn't available on this operating system.";

pub const HEVC: bool = false;
/// No 4:4:4 either.
pub const HEVC444: bool = false;

pub fn permissions() -> Permissions {
    Permissions {
        screen: false,
        input: false,
    }
}

pub fn displays() -> anyhow::Result<Vec<DisplayInfo>> {
    bail!(NOT_YET)
}

pub fn start(
    _display: u32,
    _enc: Encoding,
    _out: UnboundedSender<Encoded>,
) -> anyhow::Result<Box<dyn Video>> {
    bail!(NOT_YET)
}

#[derive(Default)]
pub struct Input;

impl Input {
    pub fn new() -> Self {
        Self
    }
    pub fn apply(&mut self, _ev: InputEvent) {}
}
