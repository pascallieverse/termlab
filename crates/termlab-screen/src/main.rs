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
//! - `permissions`: prints whether screen capture and input are allowed, as JSON.

// Other platforms have a stub for now, which leaves the shared types unused there.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod platform;
mod proto;
mod session;

use std::path::PathBuf;

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
            println!("{}", serde_json::to_string(&platform::permissions())?);
            Ok(())
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
        other => anyhow::bail!("unknown command: {other:?} (serve, displays, permissions, probe)"),
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
