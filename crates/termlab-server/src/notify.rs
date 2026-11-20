//! What happens when a session changes: signed webhooks, phone (web push) notifications,
//! and the audit log.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair};
use web_push_native::p256::PublicKey;
use web_push_native::{Auth, WebPushBuilder};

use crate::AppRef;
use crate::util::{clip, now, unix_now};

/// Event name → description, in the order the web UI lists them.
pub const EVENTS: [(&str, &str); 6] = [
    ("session.needs_you", "A session waits for input"),
    (
        "approval.requested",
        "An agent asks to run a command or edit a file",
    ),
    ("session.done", "A session finished its task"),
    ("session.error", "A session exited with an error"),
    ("session.started", "A new session was opened"),
    ("machine.offline", "A paired computer stopped reporting"),
];
pub const TEST_EVENT: &str = "webhook.test";
/// After this many failed deliveries in a row an endpoint shows as failing.
pub const FAILING_AFTER: i64 = 3;
/// Seconds before attempt 2, 3, 4 and 5.
const BACKOFF: [u64; 4] = [10, 60, 300, 1800];
const PUSH_EVERY: Duration = Duration::from_secs(60);

// ---- audit -----------------------------------------------------------------------------

pub fn audit(
    app: &AppRef,
    action: &str,
    detail: Option<&str>,
    machine: Option<&str>,
    title: Option<&str>,
    ip: &str,
) {
    let _ = app.db.with(|c| {
        c.execute(
            "insert into audit_log (action, detail, machine, session_title, ip, created_at) values (?1, ?2, ?3, ?4, ?5, ?6)",
            (action, detail.map(|d| clip(d, 500)), machine, title, ip, now()),
        )
    });
}

// ---- events ----------------------------------------------------------------------------

/// A session event: webhooks, and a push for the ones that need the person.
pub fn event(app: &AppRef, event: &str, session: &Value) {
    let reason = match event {
        "session.needs_you" if !session["approval"].is_null() => Some("approval_requested"),
        "session.needs_you" => Some("waiting_for_input"),
        "approval.requested" => Some("approval_requested"),
        "session.error" => Some("exited_with_error"),
        "session.done" => Some("finished"),
        _ => None,
    };
    let mut data = json!({
        "machine": session["machine"],
        "project": session["project"],
        "agent": session["agent"],
        "session": {
            "id": session["id"], "title": session["title"],
            "status": session["status"], "branch": session["branch"],
        },
    });
    if let Some(r) = reason {
        data["reason"] = json!(r);
    }
    if !session["approval"].is_null() {
        data["approval"] = session["approval"].clone();
    }
    dispatch(app, event, data);

    if matches!(event, "session.needs_you" | "session.error") {
        let id = session["id"].as_str().unwrap_or_default().to_string();
        if app.push_throttle.allow(&id) {
            let agent = match session["agent"].as_str().unwrap_or("") {
                "claude" => "Claude Code",
                "codex" => "Codex CLI",
                "vibe" => "Mistral Vibe",
                "antigravity" | "agy" => "Antigravity CLI",
                other => other,
            };
            let title = if event == "session.error" {
                format!("{agent} stopped with an error")
            } else {
                format!("{agent} needs you")
            };
            let mut body = format!(
                "{} · {}",
                session["project"].as_str().unwrap_or(""),
                session["title"].as_str().unwrap_or("")
            );
            if let Some(d) = session["approval"]["detail"].as_str() {
                body.push_str(" — ");
                body.push_str(d);
            }
            push_all(
                app,
                json!({ "title": title, "body": clip(&body, 180), "url": format!("/remote?session={id}") }),
            );
        }
    }
}

pub fn machine_offline(app: &AppRef, name: &str, last_seen: Option<String>) {
    dispatch(
        app,
        "machine.offline",
        json!({ "machine": name, "reason": "no_heartbeat", "last_seen_at": last_seen }),
    );
}

// ---- webhooks --------------------------------------------------------------------------

/// Queues a delivery to every enabled endpoint that listens to the event.
pub fn dispatch(app: &AppRef, event: &str, data: Value) {
    let endpoints: Vec<i64> = app
        .db
        .with(|c| {
            let mut st = c.prepare("select id, events from webhook_endpoints where enabled = 1")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, events)| {
            serde_json::from_str::<Vec<String>>(events).is_ok_and(|e| e.iter().any(|x| x == event))
        })
        .map(|(id, _)| id)
        .collect();
    for id in endpoints {
        send(app, id, event, data.clone());
    }
}

/// Creates a delivery for one endpoint and sends it in the background.
pub fn send(app: &AppRef, endpoint: i64, event: &str, data: Value) -> Option<i64> {
    let event_id = uuid::Uuid::new_v4().to_string();
    let mut payload =
        json!({ "id": event_id, "event": event, "created_at": now(), "program": "termlab" });
    if let (Value::Object(p), Value::Object(d)) = (&mut payload, data) {
        p.extend(d);
    }
    queue(app, endpoint, &event_id, event, &payload)
}

/// The same payload again, as a new delivery.
pub fn redeliver(app: &AppRef, delivery: i64) -> Option<i64> {
    let (endpoint, event_id, event, payload): (i64, String, String, String) = app
        .db
        .with(|c| {
            c.query_row(
                "select endpoint_id, event_id, event, payload from webhook_deliveries where id = ?1",
                [delivery],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .ok()?;
    let payload: Value = serde_json::from_str(&payload).ok()?;
    queue(app, endpoint, &event_id, &event, &payload)
}

fn queue(app: &AppRef, endpoint: i64, event_id: &str, event: &str, payload: &Value) -> Option<i64> {
    let id = app
        .db
        .with(|c| {
            c.execute(
                "insert into webhook_deliveries (endpoint_id, event_id, event, payload, created_at) values (?1, ?2, ?3, ?4, ?5)",
                (endpoint, event_id, event, payload.to_string(), now()),
            )?;
            Ok(c.last_insert_rowid())
        })
        .ok()?;
    let app = app.clone();
    tokio::spawn(async move { deliver(app, id).await });
    Some(id)
}

/// `t=<unix>,v1=<hex HMAC-SHA256 of "<t>.<body>">`
pub fn signature(secret: &str, body: &str, t: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(format!("{t}.{body}").as_bytes());
    format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
}

async fn deliver(app: AppRef, delivery: i64) {
    for attempt in 1..=5u32 {
        let row: Option<(String, String, String, String, String, bool)> = app
            .db
            .with(|c| {
                c.query_row(
                    "select e.url, e.secret, d.event, d.event_id, d.payload, e.enabled
                     from webhook_deliveries d join webhook_endpoints e on e.id = d.endpoint_id where d.id = ?1",
                    [delivery],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get::<_, i64>(5)? != 0)),
                )
            })
            .ok();
        let Some((url, secret, event, event_id, body, enabled)) = row else {
            return;
        };
        if !enabled {
            record(&app, delivery, attempt, "disabled", "", 0, false);
            return;
        }
        let target = match check_url(&url, app.cfg.allow_private_webhooks).await {
            Ok(t) => t,
            Err(reason) => {
                record(&app, delivery, attempt, "blocked", &reason, 0, false);
                return;
            }
        };
        let started = Instant::now();
        let result = async {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Termlab-Server-Webhooks/1")
                // Connect to the address that was checked, so DNS can't change in between.
                .resolve(&target.0, target.1)
                .build()?;
            client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("X-Termlab-Event", &event)
                .header("X-Termlab-Delivery", &event_id)
                .header("X-Termlab-Signature", signature(&secret, &body, unix_now()))
                .body(body.clone())
                .send()
                .await
        }
        .await;
        let ms = started.elapsed().as_millis() as i64;
        let ok = match result {
            Ok(res) => {
                let status = res.status();
                let text = res.text().await.unwrap_or_default();
                record(
                    &app,
                    delivery,
                    attempt,
                    status.as_str(),
                    &text,
                    ms,
                    status.is_success(),
                );
                status.is_success()
            }
            Err(e) => {
                let code = if e.is_timeout() { "timeout" } else { "error" };
                record(&app, delivery, attempt, code, &e.to_string(), ms, false);
                false
            }
        };
        if ok || attempt == 5 {
            return;
        }
        tokio::time::sleep(Duration::from_secs(BACKOFF[(attempt - 1) as usize])).await;
    }
}

fn record(app: &AppRef, delivery: i64, attempt: u32, code: &str, body: &str, ms: i64, ok: bool) {
    let t = now();
    let _ = app.db.with(|c| {
        c.execute(
            "update webhook_deliveries set attempt = ?1, response_code = ?2, response_body = ?3,
               duration_ms = ?4, ok = ?5, delivered_at = ?6 where id = ?7",
            (attempt, code, clip(body, 2000), ms, ok, &t, delivery),
        )?;
        c.execute(
            "update webhook_endpoints set last_delivery_at = ?1,
               consecutive_failures = case when ?2 then 0 else consecutive_failures + 1 end
             where id = (select endpoint_id from webhook_deliveries where id = ?3)",
            (&t, ok, delivery),
        )
    });
}

/// Checks a webhook URL and resolves its host. Returns (host, address to connect to).
pub async fn check_url(url: &str, allow_private: bool) -> Result<(String, SocketAddr), String> {
    let parsed = url::Url::parse(url).map_err(|_| "Not a valid URL".to_string())?;
    let scheme_ok = parsed.scheme() == "https" || (allow_private && parsed.scheme() == "http");
    let Some(host) = parsed.host_str().filter(|_| scheme_ok) else {
        return Err("Only https URLs are allowed".into());
    };
    let host = host.trim_matches(['[', ']']).to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| "Host does not resolve".to_string())?
        .collect();
    if addrs.is_empty() {
        return Err("Host does not resolve".into());
    }
    if !allow_private && addrs.iter().any(|a| !is_public(a.ip())) {
        return Err("Host resolves to a private address".into());
    }
    Ok((host, addrs[0]))
}

pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // carrier-grade NAT
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80) // link local
        }
    }
}

// ---- web push --------------------------------------------------------------------------

/// The server's VAPID key, kept in the data dir.
pub fn load_vapid(dir: &std::path::Path) -> anyhow::Result<ES256KeyPair> {
    let path = dir.join("vapid.key");
    if let Ok(raw) = std::fs::read_to_string(&path) {
        let bytes = URL_SAFE_NO_PAD.decode(raw.trim())?;
        return ES256KeyPair::from_bytes(&bytes).map_err(|e| anyhow::anyhow!("vapid.key: {e}"));
    }
    let kp = ES256KeyPair::generate();
    crate::write_private(&path, URL_SAFE_NO_PAD.encode(kp.to_bytes()).as_bytes())?;
    Ok(kp)
}

/// The public key browsers subscribe with (uncompressed point, base64url).
pub fn vapid_public(kp: &ES256KeyPair) -> String {
    URL_SAFE_NO_PAD.encode(kp.public_key().public_key().to_bytes_uncompressed())
}

#[derive(Default)]
pub struct PushThrottle(Mutex<HashMap<String, Instant>>);

impl PushThrottle {
    pub fn allow(&self, key: &str) -> bool {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, t| t.elapsed() < PUSH_EVERY);
        if map.contains_key(key) {
            return false;
        }
        map.insert(key.to_string(), Instant::now());
        true
    }
}

/// Sends a notification to every subscribed device.
pub fn push_all(app: &AppRef, message: Value) {
    let subs: Vec<(i64, String, String, String)> = app
        .db
        .with(|c| {
            let mut st = c.prepare("select id, endpoint, p256dh, auth from push_subscriptions")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
        .unwrap_or_default();
    for (id, endpoint, p256dh, auth) in subs {
        let app = app.clone();
        let body = message.to_string();
        tokio::spawn(async move {
            match push_one(&app, &endpoint, &p256dh, &auth, body).await {
                Ok(status) if status == 404 || status == 410 => {
                    // The browser dropped the subscription.
                    let _ = app
                        .db
                        .with(|c| c.execute("delete from push_subscriptions where id = ?1", [id]));
                }
                Ok(status) if !(200..300).contains(&status) => {
                    tracing::warn!(status, "push rejected")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "push failed"),
            }
        });
    }
}

async fn push_one(
    app: &AppRef,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    body: String,
) -> anyhow::Result<u16> {
    let ua_public =
        PublicKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(p256dh.trim_end_matches('='))?)?;
    let auth_bytes = URL_SAFE_NO_PAD.decode(auth.trim_end_matches('='))?;
    anyhow::ensure!(auth_bytes.len() == 16, "bad auth secret");
    let ua_auth = Auth::clone_from_slice(&auth_bytes);
    let contact = format!(
        "mailto:termlab@{}",
        url::Url::parse(&app.cfg.public_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "localhost".into())
    );
    let req = WebPushBuilder::new(endpoint.parse()?, ua_public, ua_auth)
        .with_valid_duration(Duration::from_secs(3600))
        .with_vapid(&app.vapid, &contact)
        .build(body)?;
    let (parts, body) = req.into_parts();
    let mut out = app.http.post(endpoint).body(body);
    for (k, v) in parts.headers.iter() {
        out = out.header(k.as_str(), v.as_bytes());
    }
    Ok(out.send().await?.status().as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_hmac_of_time_and_body() {
        let sig = signature("whsec_x", "{\"a\":1}", 1_700_000_000);
        let mut mac = Hmac::<Sha256>::new_from_slice(b"whsec_x").unwrap();
        mac.update(b"1700000000.{\"a\":1}");
        assert_eq!(
            sig,
            format!(
                "t=1700000000,v1={}",
                hex::encode(mac.finalize().into_bytes())
            )
        );
    }

    #[test]
    fn private_addresses_are_not_public() {
        for ip in [
            "10.0.0.5",
            "127.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "1.1.1.1", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn webhook_urls_must_be_public_https() {
        assert!(check_url("http://example.com/x", false).await.is_err());
        assert!(check_url("https://127.0.0.1/x", false).await.is_err());
        assert!(check_url("https://127.0.0.1/x", true).await.is_ok());
        assert!(check_url("http://127.0.0.1:9/x", true).await.is_ok());
        assert!(check_url("not a url", true).await.is_err());
    }

    #[test]
    fn push_throttle_allows_one_per_minute_per_session() {
        let t = PushThrottle::default();
        assert!(t.allow("a"));
        assert!(!t.allow("a"));
        assert!(t.allow("b"));
    }

    #[test]
    fn vapid_key_is_stored_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_vapid(dir.path()).unwrap();
        let b = load_vapid(dir.path()).unwrap();
        assert_eq!(vapid_public(&a), vapid_public(&b));
        assert_eq!(URL_SAFE_NO_PAD.decode(vapid_public(&a)).unwrap().len(), 65);
    }
}
