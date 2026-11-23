//! Platforms without screen sharing yet: everything reports "not supported".

use anyhow::bail;
use tokio::sync::mpsc::UnboundedSender;

use super::Permissions;
use crate::proto::{DisplayInfo, Encoded, InputEvent, Quality};

const NOT_YET: &str = "Screen sharing isn't available on this operating system yet.";

pub fn permissions() -> Permissions {
    Permissions {
        screen: false,
        input: false,
    }
}

pub fn displays() -> anyhow::Result<Vec<DisplayInfo>> {
    bail!(NOT_YET)
}

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub struct Stream;

impl Stream {
    pub fn start(
        _display: u32,
        _quality: Quality,
        _out: UnboundedSender<Encoded>,
    ) -> anyhow::Result<Self> {
        bail!(NOT_YET)
    }
    pub fn display(&self) -> u32 {
        0
    }
    pub fn bounds(&self) -> Bounds {
        Bounds {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }
    pub fn ack(&self) {}
    pub fn keyframe(&self) {}
    pub fn refine(&self) {}
    pub fn reset_window(&self) {}
    pub fn inflight(&self) -> u32 {
        0
    }
}

#[derive(Default)]
pub struct Input;

impl Input {
    pub fn new() -> Self {
        Self
    }
    pub fn apply(&mut self, _ev: InputEvent) {}
}

impl Drop for Stream {
    /// Nothing to stop; here so the session code can stop a stream the same way everywhere.
    fn drop(&mut self) {}
}
