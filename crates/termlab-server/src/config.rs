//! Command line and environment settings.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "termlab-server",
    version,
    about = "Your own Termlab server: remote access to your Termlab computers, and webhooks"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server.
    Serve(ServeArgs),
    /// Remove the owner account (password and authenticator). The next start prints a
    /// new setup token, so you can create it again. Paired computers stay paired.
    ResetOwner(DataArgs),
}

#[derive(Debug, Clone, Args)]
pub struct DataArgs {
    /// Where the database and keys are kept.
    #[arg(long, env = "TERMLAB_DATA_DIR", default_value = "./termlab-data")]
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub data: DataArgs,

    /// Address to listen on. Keep it on localhost and put a reverse proxy with HTTPS in front.
    #[arg(long, env = "TERMLAB_LISTEN", default_value = "127.0.0.1:8484")]
    pub listen: SocketAddr,

    /// The address people and Termlab use to reach this server, e.g. https://remote.example.com.
    /// Decides whether cookies are marked Secure, and which origin may open websockets.
    #[arg(long, env = "TERMLAB_PUBLIC_URL")]
    pub public_url: String,

    /// Believe X-Forwarded-For / X-Real-IP from a reverse proxy on this machine or a private
    /// network (loopback, 10/8, 172.16/12, 192.168/16, as with Docker).
    #[arg(long, env = "TERMLAB_TRUST_PROXY", default_value_t = true, action = clap::ArgAction::Set)]
    pub trust_proxy: bool,

    /// Let webhooks go to private and loopback addresses (for a receiver on your own network).
    #[arg(long, env = "TERMLAB_ALLOW_PRIVATE_WEBHOOKS", default_value_t = false)]
    pub allow_private_webhooks: bool,

    /// A name for this server, shown in the web UI and to Termlab when pairing.
    #[arg(long, env = "TERMLAB_SERVER_NAME", default_value = "Termlab Server")]
    pub server_name: String,
}

/// Settings the running server uses.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub listen: SocketAddr,
    /// Without a trailing slash.
    pub public_url: String,
    pub secure_cookies: bool,
    pub trust_proxy: bool,
    pub allow_private_webhooks: bool,
    pub server_name: String,
}

impl Config {
    pub fn from_args(a: ServeArgs) -> anyhow::Result<Self> {
        let public_url = a.public_url.trim().trim_end_matches('/').to_string();
        let parsed = url::Url::parse(&public_url).map_err(|_| {
            anyhow::anyhow!("--public-url must be a full URL like https://remote.example.com")
        })?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            anyhow::bail!("--public-url must start with https:// (or http:// for local testing)");
        }
        Ok(Self {
            data_dir: a.data.data_dir,
            listen: a.listen,
            secure_cookies: parsed.scheme() == "https",
            public_url,
            trust_proxy: a.trust_proxy,
            allow_private_webhooks: a.allow_private_webhooks,
            server_name: a.server_name,
        })
    }

    /// `https://host[:port]`, for checking a websocket's Origin header.
    pub fn origin(&self) -> String {
        url::Url::parse(&self.public_url)
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_else(|_| self.public_url.clone())
    }
}
