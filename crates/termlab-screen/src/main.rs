//! `termlab-screen`: shares this computer's screen with your own Termlab server, and takes
//! mouse and keyboard input from the browser watching it.
//!
//! `termlabd` starts it when a signed-in browser opens a screen session (and the person
//! switched screen sharing on in Termlab's settings). It reads one JSON line from stdin
//! ([`session::Launch`]: the server, this machine's token and the session id), connects its
//! own websocket to the server and streams until the browser leaves, sharing is switched
//! off, or the connection drops. One process per screen session.
//!
//! Subcommands:
//! - `serve`: the above.
//! - `displays`: prints the displays as JSON (for checks).
//! - `probe <seconds> <out.h264>`: captures the main display into an H.264 file and prints
//!   what it got (for development: `ffprobe` can read the file).
//! - `permissions [data-dir]`: prints whether screen capture and input are allowed, as JSON.
//! - `authorize <data-dir>`: Linux under Wayland: asks the desktop now and keeps the approval,
//!   so later sessions start without anyone at the computer. Elsewhere: nothing to do.

// Platforms without screen sharing have a stub, which leaves the shared types unused there.
#![cfg_attr(
    not(any(target_os = "macos", windows, target_os = "linux")),
    allow(dead_code)
)]

mod platform;
mod proto;
mod session;
mod system;

use std::path::PathBuf;

use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    match cmd.as_str() {
        "serve" => {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()?;
            rt.block_on(session::serve())
        }
        "displays" => {
            let list = platform::displays()?;
            println!("{}", serde_json::to_string_pretty(&list)?);
            Ok(())
        }
        "permissions" => {
            if let Some(dir) = args.next() {
                platform::set_data_dir(&PathBuf::from(dir));
            }
            println!("{}", serde_json::to_string(&platform::permissions())?);
            Ok(())
        }
        "authorize" => {
            let dir = PathBuf::from(args.next().context("authorize <data-dir>")?);
            platform::authorize(&dir)
        }
        "probe" => {
            let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(3);
            let out = PathBuf::from(args.next().unwrap_or_else(|| "probe.h264".into()));
            init_stderr_logging();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(session::probe(secs, out))
        }
        "--version" => {
            println!("termlab-screen {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => anyhow::bail!(
            "unknown command: {other:?} (serve, displays, permissions, authorize, probe)"
        ),
    }
}

fn init_stderr_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();
}
