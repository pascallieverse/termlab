//! Termlab core: everything except UI. Agent adapters, pty sessions, scrollback,
//! persistence and (in later milestones) status detection, discovery and the hook receiver.

pub mod adapter;
pub mod db;
pub mod env;
pub mod error;
pub mod git;
pub mod hooks;
pub mod ipc;
pub mod model;
pub mod pty;
pub mod scrollback;
pub mod service;
pub mod status;
pub mod transcript;

pub use error::{CoreError, Result};
pub use service::{CoreEvent, DataDirs, StartRequest, Termlab, TermlabConfig};
