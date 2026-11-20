//! Remote access: the link between this computer's sessions and the owner's own, self-hosted
//! Termlab server (`termlab-server`), so they can be watched and answered from a browser or
//! phone.
//!
//! The bridge itself (HTTP and websocket client) lives in `termlabd`, which is the only
//! process that needs network access. This module holds what the IPC protocol shares with
//! it: the status the app shows, and the [`RemoteControl`] calls the app can make.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

/// Where the bridge is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteState {
    /// Not linked to an account.
    Unpaired,
    /// Linked, (re)connecting to the server.
    Connecting,
    /// Linked and live: sessions are visible remotely.
    Connected,
    /// Linked, but the account has no plan with remote access.
    NoPlan,
    /// Linked, but the last attempt failed; it retries by itself.
    Error,
}

/// What Settings › Remote access shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub state: RemoteState,
    /// The Termlab server, e.g. `https://remote.example.com`.
    pub server_url: Option<String>,
    pub machine_name: Option<String>,
    /// The server's name, as it reported it when pairing.
    pub account_name: Option<String>,
    pub plan: Option<String>,
    /// Why the last attempt failed, in words for the user.
    pub error: Option<String>,
    /// How many sessions this computer currently shows remotely.
    pub sessions: usize,
    /// Browsers watching a terminal right now.
    pub viewers: usize,
    /// A name to suggest for this computer when pairing.
    pub default_machine_name: String,
}

/// Pairing details from Settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub server_url: String,
    pub code: String,
    pub machine_name: String,
}

pub type RemoteFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

/// Implemented by `termlabd`'s bridge; the IPC server forwards the app's requests to it.
pub trait RemoteControl: Send + Sync {
    fn status(&self) -> RemoteStatus;
    fn pair(&self, request: PairRequest) -> RemoteFuture<RemoteStatus>;
    fn unpair(&self) -> RemoteFuture<RemoteStatus>;
    /// Whether the bridge keeps the daemon alive (paired: phones expect the computer to
    /// stay reachable even with no sessions running).
    fn keeps_alive(&self) -> bool;
}
