//! `termlab-server`: your own server for Termlab. It gives you remote access to the
//! sessions on your Termlab computers from a browser or phone, and sends signed webhooks
//! when they change. One owner, one SQLite file, no third-party service in between.
//!
//! See docs/self-hosting.md (running it) and docs/server-protocol.md (the API).

mod api;
mod auth;
mod config;
mod db;
mod hub;
mod notify;
mod sessions;
mod util;
mod web;
mod ws;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use axum::Router;
use axum::middleware::{from_fn, from_fn_with_state};
use clap::Parser;
use tower_http::limit::RequestBodyLimitLayer;
use web_push_native::jwt_simple::algorithms::ES256KeyPair;

use crate::config::{Cli, Command, Config};
use crate::db::Db;

pub type AppRef = Arc<App>;

/// First-run state: the setup token printed at start, and the authenticator being set up.
pub struct SetupState {
    pub token: String,
    pub totp_secret: Option<String>,
}

pub struct App {
    pub cfg: Config,
    pub db: Db,
    pub hub: hub::Hub,
    pub setup: Mutex<Option<SetupState>>,
    /// Authenticator re-enrolment in progress, per web session.
    pub pending_totp: Mutex<HashMap<i64, String>>,
    pub limiter: auth::LoginLimiter,
    pub push_throttle: notify::PushThrottle,
    pub vapid: ES256KeyPair,
    /// For web push. Webhooks build their own client per delivery (pinned address).
    pub http: reqwest::Client,
}

fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => {
            let cfg = Config::from_args(args)?;
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(serve(cfg))
        }
        Command::ResetOwner(args) => {
            let db = Db::open(&args.data_dir.join("termlab-server.sqlite"))?;
            db.with(|c| c.execute_batch("delete from owner; delete from web_sessions;"))?;
            println!(
                "The owner account was removed. Restart the server: it prints a new setup token."
            );
            Ok(())
        }
    }
}

/// Writes a file only the server's user can read.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    opts.open(path)?.write_all(bytes)
}

pub fn build_app(cfg: Config) -> anyhow::Result<AppRef> {
    std::fs::create_dir_all(&cfg.data_dir)
        .with_context(|| format!("creating {}", cfg.data_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&cfg.data_dir, std::fs::Permissions::from_mode(0o700));
    }
    let db =
        Db::open(&cfg.data_dir.join("termlab-server.sqlite")).context("opening the database")?;
    let vapid = notify::load_vapid(&cfg.data_dir).context("loading the push key")?;
    let setup = (!db.has_owner()).then(|| SetupState {
        token: util::token(18),
        totp_secret: None,
    });
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    Ok(Arc::new(App {
        cfg,
        db,
        hub: hub::Hub::default(),
        setup: Mutex::new(setup),
        pending_totp: Mutex::new(HashMap::new()),
        limiter: auth::LoginLimiter::default(),
        push_throttle: notify::PushThrottle::default(),
        vapid,
        http,
    }))
}

pub fn router(app: AppRef) -> Router {
    api::routes()
        .merge(ws::routes())
        .fallback(web::static_files)
        .layer(from_fn_with_state(app.clone(), auth::csrf_guard))
        .layer(RequestBodyLimitLayer::new(1 << 20))
        .layer(from_fn(web::security_headers))
        .with_state(app)
}

async fn serve(cfg: Config) -> anyhow::Result<()> {
    let listen = cfg.listen;
    let app = build_app(cfg)?;
    if let Some(s) = app.setup.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let line = "─".repeat(64);
        println!(
            "\n{line}\n  Termlab Server has no owner yet.\n  Open {} and enter this setup token:\n\n      {}\n{line}\n",
            app.cfg.public_url, s.token
        );
    }
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("listening on {listen}"))?;
    tracing::info!(%listen, public_url = %app.cfg.public_url, "termlab-server started");
    axum::serve(
        listener,
        router(app).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal");
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
        tracing::info!("stopping");
    })
    .await?;
    Ok(())
}
