//! The embedded web UI (apps/server-web, built into `dist/`) and security headers.

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../apps/server-web/dist"]
#[allow_missing = true]
struct Assets;

const MISSING: &str = "<!doctype html><meta charset=utf-8><title>Termlab Server</title>\
<p style=\"font-family:system-ui;padding:2rem\">The web UI wasn't built into this binary. \
Run <code>npm ci &amp;&amp; npm run build</code> in <code>apps/server-web</code>, then build the server again.</p>";

/// Serves a file from the UI, or index.html for app routes (it routes in the browser).
pub async fn static_files(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "message": "Not found." })),
        )
            .into_response();
    }
    if !path.is_empty()
        && let Some(file) = Assets::get(path)
    {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return (
            [
                (CONTENT_TYPE, mime.as_ref().to_string()),
                (CACHE_CONTROL, cache.to_string()),
            ],
            Body::from(file.data),
        )
            .into_response();
    }
    match Assets::get("index.html") {
        Some(index) => (
            [
                (CONTENT_TYPE, "text/html; charset=utf-8"),
                (CACHE_CONTROL, "no-cache"),
            ],
            Body::from(index.data),
        )
            .into_response(),
        None => ([(CONTENT_TYPE, "text/html; charset=utf-8")], MISSING).into_response(),
    }
}

/// Headers for every response: no framing, no sniffing, no referrers, and a strict CSP.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, k: &'static str, v: &'static str| {
        h.insert(k, HeaderValue::from_static(v));
    };
    set(
        h,
        "content-security-policy",
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
         font-src 'self' data:; connect-src 'self'; worker-src 'self'; manifest-src 'self'; \
         frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    );
    set(h, "x-frame-options", "DENY");
    set(h, "x-content-type-options", "nosniff");
    set(h, "referrer-policy", "no-referrer");
    set(
        h,
        "permissions-policy",
        "camera=(), microphone=(), geolocation=()",
    );
    res
}
