//! The owner's sign-in: password (argon2id) plus an authenticator app (TOTP), web sessions
//! in an HttpOnly cookie, a CSRF guard and a sign-in rate limit.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::{ConnectInfo, FromRequestParts, OptionalFromRequestParts, Request, State};
use axum::http::header::{COOKIE, USER_AGENT};
use axum::http::{HeaderMap, Method, StatusCode, request::Parts};
use axum::middleware::Next;
use axum::response::Response;
use rusqlite::OptionalExtension;
use totp_rs::{Algorithm, Secret, TOTP};

use crate::util::{ApiError, now, sha256_hex, token};
use crate::{AppRef, util};

pub const COOKIE_NAME: &str = "termlab_session";
/// Every request that changes something carries this header. A cross-site form can't set
/// custom headers, so this (with SameSite=Strict cookies) stops CSRF.
pub const CSRF_HEADER: &str = "x-termlab-request";
const SESSION_DAYS: i64 = 30;
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
const LOGIN_MAX_FAILURES: u32 = 5;
pub const MIN_PASSWORD: usize = 12;

// ---- passwords -------------------------------------------------------------------------

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hashing failed: {e}"))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

// ---- TOTP ------------------------------------------------------------------------------

pub fn new_totp_secret() -> String {
    Secret::generate_secret().to_encoded().to_string()
}

pub fn totp(secret_b32: &str, account: &str) -> Option<TOTP> {
    let bytes = Secret::Encoded(secret_b32.to_string()).to_bytes().ok()?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("Termlab".to_string()),
        account.to_string(),
    )
    .ok()
}

/// Accepts the current code and one step either side (clock drift).
pub fn check_totp(secret_b32: &str, code: &str) -> bool {
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    code.len() == 6
        && totp(secret_b32, "owner").is_some_and(|t| t.check_current(&code).unwrap_or(false))
}

/// What the web UI shows to enrol an authenticator app.
pub fn enrolment(secret_b32: &str, account: &str) -> serde_json::Value {
    let url = totp(secret_b32, account)
        .map(|t| t.get_url())
        .unwrap_or_default();
    let svg = qrcode::QrCode::new(url.as_bytes())
        .map(|q| {
            q.render::<qrcode::render::svg::Color>()
                .min_dimensions(200, 200)
                .dark_color(qrcode::render::svg::Color("#0b0d11"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .build()
        })
        .unwrap_or_default();
    // The UI injects this SVG; make sure it's only the QR code it should be.
    let svg = svg
        .find("<svg")
        .map(|i| svg[i..].to_string())
        .unwrap_or_default();
    serde_json::json!({ "totp_secret": secret_b32, "otpauth_url": url, "qr_svg": svg })
}

// ---- sessions --------------------------------------------------------------------------

pub fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers.get_all(COOKIE).iter().find_map(|v| {
        v.to_str().ok()?.split(';').find_map(|part| {
            let (k, v) = part.trim().split_once('=')?;
            (k == COOKIE_NAME && !v.is_empty()).then(|| v.to_string())
        })
    })
}

pub fn set_cookie(app: &AppRef, value: &str, max_age_days: i64) -> String {
    format!(
        "{COOKIE_NAME}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        max_age_days * 86400,
        if app.cfg.secure_cookies {
            "; Secure"
        } else {
            ""
        }
    )
}

pub fn clear_cookie(app: &AppRef) -> String {
    set_cookie(app, "", 0)
}

/// Starts a web session and returns the cookie value.
pub fn start_session(app: &AppRef, user_agent: &str, ip: &str) -> Result<String, ApiError> {
    let raw = token(32);
    let t = now();
    app.db.with(|c| {
        c.execute(
            "insert into web_sessions (token_hash, user_agent, ip, created_at, last_seen_at)
             values (?1, ?2, ?3, ?4, ?4)",
            (sha256_hex(&raw), util::clip(user_agent, 300), ip, &t),
        )
    })?;
    Ok(raw)
}

/// The signed-in owner, from the session cookie.
#[derive(Debug, Clone)]
pub struct Owner {
    pub session_id: i64,
    pub ip: String,
}

impl FromRequestParts<AppRef> for Owner {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &AppRef) -> Result<Self, Self::Rejection> {
        let ip = client_ip(app, parts);
        let raw =
            cookie_value(&parts.headers).ok_or_else(|| ApiError::unauthorized("Sign in first."))?;
        let hash = sha256_hex(&raw);
        let found: Option<(i64, String)> = app.db.with(|c| {
            c.query_row(
                "select id, created_at from web_sessions where token_hash = ?1",
                [&hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })?;
        let Some((id, created_at)) = found else {
            return Err(ApiError::unauthorized("Sign in first."));
        };
        let expired = time::OffsetDateTime::parse(
            &created_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map(|t| time::OffsetDateTime::now_utc() - t > time::Duration::days(SESSION_DAYS))
        .unwrap_or(true);
        if expired {
            let _ = app
                .db
                .with(|c| c.execute("delete from web_sessions where id = ?1", [id]));
            return Err(ApiError::unauthorized(
                "Your session expired. Sign in again.",
            ));
        }
        let _ = app.db.with(|c| {
            c.execute(
                "update web_sessions set last_seen_at = ?1, ip = ?2 where id = ?3",
                (now(), &ip, id),
            )
        });
        Ok(Owner { session_id: id, ip })
    }
}

impl OptionalFromRequestParts<AppRef> for Owner {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        app: &AppRef,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(
            <Owner as FromRequestParts<AppRef>>::from_request_parts(parts, app)
                .await
                .ok(),
        )
    }
}

/// Who is asking: address and browser, for sign-in and the audit log.
#[derive(Debug, Clone)]
pub struct Client {
    pub ip: String,
    pub user_agent: String,
}

impl FromRequestParts<AppRef> for Client {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &AppRef) -> Result<Self, Self::Rejection> {
        Ok(Client {
            ip: client_ip(app, parts),
            user_agent: user_agent(&parts.headers),
        })
    }
}

/// The client's address. Behind a reverse proxy on this machine or a private network
/// (Docker), its forwarded header.
pub fn client_ip(app: &AppRef, parts: &Parts) -> String {
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    // A reverse proxy runs on this machine (loopback) or, with Docker, on its bridge network.
    let local_peer = peer.is_none_or(|p| p.is_loopback() || !crate::notify::is_public(p));
    if app.cfg.trust_proxy && local_peer {
        let forwarded = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .or_else(|| parts.headers.get("x-real-ip").and_then(|v| v.to_str().ok()))
            .and_then(|v| v.parse::<IpAddr>().ok());
        if let Some(ip) = forwarded {
            return ip.to_string();
        }
    }
    peer.map(|p| p.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

// ---- CSRF ------------------------------------------------------------------------------

/// Refuses state-changing browser requests without the CSRF header. The machine API uses a
/// bearer token, not cookies, so it doesn't need it.
/// Calls Termlab makes (bearer token or pairing code, no cookie) don't carry the header.
fn csrf_exempt(path: &str) -> bool {
    path == "/api/pair" || path == "/api/machine" || path.starts_with("/api/machine/")
}

pub async fn csrf_guard(
    State(_app): State<AppRef>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let path = req.uri().path();
    let exempt = csrf_exempt(path);
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if path.starts_with("/api/") && !exempt && !safe {
        let ok = req
            .headers()
            .get(CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "1");
        if !ok {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Missing the X-Termlab-Request header.",
            ));
        }
    }
    Ok(next.run(req).await)
}

// ---- rate limit ------------------------------------------------------------------------

#[derive(Default)]
pub struct LoginLimiter(Mutex<HashMap<String, (u32, Instant)>>);

impl LoginLimiter {
    /// `Err(seconds)` while this address has too many recent failures.
    pub fn check(&self, ip: &str) -> Result<(), u64> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(ip) {
            Some((n, since)) if *n >= LOGIN_MAX_FAILURES && since.elapsed() < LOGIN_WINDOW => {
                Err((LOGIN_WINDOW - since.elapsed()).as_secs().max(1))
            }
            _ => Ok(()),
        }
    }

    pub fn failed(&self, ip: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(ip.to_string()).or_insert((0, Instant::now()));
        if entry.1.elapsed() >= LOGIN_WINDOW {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
    }

    pub fn succeeded(&self, ip: &str) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(ip);
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn machine_calls_skip_the_csrf_header() {
        assert!(csrf_exempt("/api/pair"));
        assert!(csrf_exempt("/api/machine"));
        assert!(csrf_exempt("/api/machine/ws"));
        assert!(!csrf_exempt("/api/machines"));
        assert!(!csrf_exempt("/api/machines/1"));
        assert!(!csrf_exempt("/api/login"));
    }
    use super::*;

    #[test]
    fn passwords_hash_and_verify() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn totp_accepts_the_current_code_only() {
        let secret = new_totp_secret();
        let code = totp(&secret, "owner").unwrap().generate_current().unwrap();
        assert!(check_totp(&secret, &code));
        assert!(check_totp(
            &secret,
            &format!("{} {}", &code[..3], &code[3..])
        ));
        let wrong = format!("{:06}", (code.parse::<u32>().unwrap() + 1) % 1_000_000);
        assert!(!check_totp(&secret, &wrong));
        assert!(!check_totp(&secret, "12345"));
    }

    #[test]
    fn enrolment_has_a_qr_svg() {
        let e = enrolment(&new_totp_secret(), "owner");
        assert!(e["qr_svg"].as_str().unwrap().starts_with("<svg"));
        assert!(
            e["otpauth_url"]
                .as_str()
                .unwrap()
                .starts_with("otpauth://totp/")
        );
    }

    #[test]
    fn limiter_blocks_after_five_failures() {
        let l = LoginLimiter::default();
        for _ in 0..5 {
            assert!(l.check("1.2.3.4").is_ok());
            l.failed("1.2.3.4");
        }
        assert!(l.check("1.2.3.4").is_err());
        assert!(l.check("5.6.7.8").is_ok());
        l.succeeded("1.2.3.4");
        assert!(l.check("1.2.3.4").is_ok());
    }

    #[test]
    fn reads_the_session_cookie() {
        let mut h = HeaderMap::new();
        h.insert(COOKIE, "a=1; termlab_session=abc; b=2".parse().unwrap());
        assert_eq!(cookie_value(&h).as_deref(), Some("abc"));
    }
}
