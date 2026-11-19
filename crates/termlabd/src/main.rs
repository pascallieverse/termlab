//! `termlabd`: the per-user background process that owns Termlab's agent sessions, so they
//! keep running when the app window (or the whole app) is closed.
//!
//! Usage: `termlabd [--data-dir <dir>]`. Listens on `<data-dir>/termlabd.sock` for the app
//! and on `<data-dir>/hook.sock` for `termlab-hook` (on Windows: named pipes derived from
//! those paths). Exits on SIGTERM/SIGINT (saving every session's output first), on an app's
//! `Shutdown` request, or after ten idle minutes with no sessions and no app connected.
//! While the computer is paired for remote access it stays up, so phones can reach it.

mod remote;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use termlab_core::env::UserEnv;
use termlab_core::ipc::Server;
use termlab_core::remote::RemoteControl;
use termlab_core::{DataDirs, Termlab, TermlabConfig};

const IDLE_EXIT: Duration = Duration::from_secs(10 * 60);

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut data_dir: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            "--version" => {
                println!("termlabd {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    let dirs = match data_dir {
        Some(d) => DataDirs::new(d),
        None => DataDirs::for_user()?,
    };
    std::fs::create_dir_all(&dirs.root)?;
    init_logging(&dirs)?;

    // TLS for remote access (reqwest and the websocket) uses ring.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(dirs))
}

fn init_logging(dirs: &DataDirs) -> anyhow::Result<()> {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dirs.root.join("termlabd.log"))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(log))
        .init();
    Ok(())
}

async fn run(dirs: DataDirs) -> anyhow::Result<()> {
    let socket = dirs.daemon_socket();
    if termlab_core::transport::connect(&socket).await.is_ok() {
        eprintln!("termlabd is already running ({})", socket.display());
        return Ok(());
    }

    // `termlab-hook` ships next to this binary.
    let hook_binary = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            let name = format!("termlab-hook{}", std::env::consts::EXE_SUFFIX);
            exe.parent().map(|d| d.join(name))
        })
        .filter(|p| p.exists());
    if hook_binary.is_none() {
        tracing::warn!(
            "termlab-hook not found next to termlabd; status falls back to screen heuristics"
        );
    }

    let env = tokio::task::spawn_blocking(UserEnv::capture_login_shell).await?;
    let mut config = TermlabConfig::new(dirs.clone(), env);
    config.hook_binary = hook_binary;
    let tl = Termlab::open(config).context("opening the Termlab database")?;
    tl.start_background()?;
    {
        // Detect agents up front so the app's first request is fast.
        let tl = tl.clone();
        tokio::task::spawn_blocking(move || tl.refresh_agents());
    }

    let server = Server::new(tl.clone());
    let bridge = remote::Bridge::new(tl.clone(), &dirs).context("setting up remote access")?;
    let remote: Arc<dyn RemoteControl> = Arc::new(remote::RemoteHandle(bridge.clone()));
    let _ = server.remote.set(remote.clone());
    bridge.start();
    tracing::info!(pid = std::process::id(), socket = %socket.display(), "termlabd started");
    {
        let server = server.clone();
        let socket = socket.clone();
        tokio::spawn(async move {
            if let Err(e) = server.serve(&socket).await {
                tracing::error!(error = %e, "socket server stopped");
            }
        });
    }

    tokio::select! {
        _ = terminated() => tracing::info!("SIGTERM"),
        _ = tokio::signal::ctrl_c() => tracing::info!("SIGINT"),
        _ = server.shutdown.notified() => tracing::info!("shutdown requested by the app"),
        _ = idle_watch(server.clone(), remote) => tracing::info!("idle with no sessions or clients; exiting"),
    }
    let tl2 = tl.clone();
    tokio::task::spawn_blocking(move || tl2.shutdown()).await?;
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(dirs.hook_socket());
    tracing::info!("termlabd stopped");
    Ok(())
}

/// Resolves when the process is asked to stop (SIGTERM). Windows has no such signal: there
/// the daemon stops on the app's request, on Ctrl-C, or when idle.
async fn terminated() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            sigterm.recv().await;
            return;
        }
    }
    std::future::pending::<()>().await;
}

async fn idle_watch(server: Arc<Server>, remote: Arc<dyn RemoteControl>) {
    let mut idle_for = Duration::ZERO;
    let step = Duration::from_secs(30);
    loop {
        tokio::time::sleep(step).await;
        if server.tl.live_count() == 0
            && server.clients.load(Ordering::SeqCst) == 0
            && !remote.keeps_alive()
        {
            idle_for += step;
            if idle_for >= IDLE_EXIT {
                return;
            }
        } else {
            idle_for = Duration::ZERO;
        }
    }
}
