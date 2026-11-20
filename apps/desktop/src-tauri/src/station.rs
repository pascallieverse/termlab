//! Stations: YouTube's embeddable player, in a child webview of the main window placed over
//! the music card's player slot.
//!
//! It can't be an iframe in Termlab's own page. YouTube only plays embeds whose request
//! names the site embedding them (the Referer header; without it the player shows error
//! 153, "embedder.identity.missing.referrer"), and Termlab's page is `tauri://localhost`,
//! which WebKit never sends as a Referer. So the player gets a small page of its own that
//! sits on an https address:
//!
//! - macOS and Linux: the page is loaded from a string with an https base URL, the app's
//!   identifier (what YouTube asks of apps that embed it in a web view). Nothing is fetched
//!   from that address.
//! - Windows: Tauri serves custom protocols there on `https://<scheme>.localhost`, so the
//!   page comes from one of those.
//!
//! The page has no access to Termlab (Tauri rejects IPC from remote origins unless a
//! capability grants it, and none does). The player inside it is sandboxed so it can't
//! navigate the page away. Termlab talks to the player through the page: commands go in
//! with `eval`, and the player's messages are queued there and collected by polling.

use std::sync::Mutex;

use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Rect, Url, WebviewUrl};

use crate::ytmusic::{DockRect, eval_json, open_in_browser, page_origin};

const LABEL: &str = "station";
const MAIN_WINDOW: &str = "main";
const PLAYER_ORIGIN: &str = "https://www.youtube-nocookie.com";

/// Where the player page lives.
#[cfg(not(windows))]
const PAGE_URL: &str = "https://dev.termlab.desktop/";
#[cfg(windows)]
const PAGE_URL: &str = "https://termlab-station.localhost/";
#[cfg(windows)]
pub const SCHEME: &str = "termlab-station";

/// The player's full size; smaller slots get it scaled down (see the page's `fit`).
const PLAYER_WIDTH: f64 = 448.0;
const PLAYER_HEIGHT: f64 = 256.0;
/// Out of view: outside the window rather than hidden, so the page keeps playing.
const AWAY: f64 = -20_000.0;

/// Where the frontend last asked for the view (logical x, y, width, height), or None for out
/// of view. Kept so a view created later appears in the right place.
static PLACE: Mutex<Option<[f64; 4]>> = Mutex::new(None);

type CmdResult<T> = Result<T, String>;

/// The player address to load: an embed on YouTube's privacy-enhanced domain, told which
/// page embeds it (the player answers postMessage only to that origin).
fn player_src(src: &str) -> CmdResult<String> {
    let mut url = Url::parse(src).map_err(|e| format!("invalid player address: {e}"))?;
    let ok = url.scheme() == "https"
        && url.host_str() == Some("www.youtube-nocookie.com")
        && url.path().starts_with("/embed/")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none();
    if !ok {
        return Err("not a YouTube player address".into());
    }
    let params: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "origin")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(params)
        .append_pair("origin", PAGE_URL.trim_end_matches('/'));
    Ok(url.into())
}

fn html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The page around the player. Slots smaller than the player get a full-size player scaled
/// down that ignores the mouse (a live thumbnail: the card's buttons control it), so
/// YouTube's own hover controls and click-to-pause stay out of the way.
fn page_html(src: &str) -> String {
    const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Termlab station</title>
<style>
html, body { margin: 0; height: 100%; overflow: hidden; background: #000; }
iframe { border: 0; display: block; width: 100%; height: 100%; }
body.small iframe { position: absolute; top: 0; left: 0; width: %WIDTH%px; height: %HEIGHT%px; transform-origin: 0 0; pointer-events: none; }
</style></head>
<body>
<iframe id="player" src="%SRC%" title="YouTube player" allow="autoplay; encrypted-media; picture-in-picture" referrerpolicy="strict-origin-when-cross-origin" sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox allow-presentation"></iframe>
<script>
(() => {
  const ORIGIN = "%ORIGIN%";
  const frame = document.getElementById("player");
  const queue = [];
  let heard = false;
  addEventListener("message", (e) => {
    if (e.origin !== ORIGIN || e.source !== frame.contentWindow) return;
    heard = true;
    queue.push(typeof e.data === "string" ? e.data : JSON.stringify(e.data));
    if (queue.length > 200) queue.splice(0, queue.length - 200);
  });
  const post = (msg) => { if (frame.contentWindow) frame.contentWindow.postMessage(msg, ORIGIN); };
  // The player ignores messages until it has loaded: ask for its events until it answers.
  frame.addEventListener("load", () => {
    let tries = 0;
    const timer = setInterval(() => {
      if (heard || ++tries > 40) return clearInterval(timer);
      post(JSON.stringify({ event: "listening", id: 1, channel: "widget" }));
    }, 250);
  });
  const fit = () => {
    const small = innerWidth < 200;
    document.body.classList.toggle("small", small);
    frame.style.transform = small ? "scale(" + innerWidth / %WIDTH% + ")" : "";
  };
  addEventListener("resize", fit);
  fit();
  window.__termlabStation = { post, drain: () => queue.splice(0) };
})();
</script>
</body></html>"#;
    PAGE.replace("%SRC%", &html_attr(src))
        .replace("%ORIGIN%", PLAYER_ORIGIN)
        .replace("%WIDTH%", &PLAYER_WIDTH.to_string())
        .replace("%HEIGHT%", &PLAYER_HEIGHT.to_string())
}

/// Main-frame and subframe loads alike come through here (WebKit reports both). The page
/// itself and the player's own frames may load; the player can't move the page (its frame
/// is sandboxed), and anything else is refused.
fn allowed_in_view(url: &Url) -> bool {
    matches!(url.scheme(), "https" | "about" | "blob")
}

/// The only unsafe code in the workspace: Tauri has no way to load a page with a base URL,
/// so this calls WebKit directly.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn load_page(view: &tauri::Webview, _src: &str, html: String) -> CmdResult<()> {
    view.with_webview(move |platform| {
        use objc2_foundation::{NSString, NSURL};
        use objc2_web_kit::WKWebView;
        // SAFETY: on macOS the platform view is the webview's WKWebView, alive for this call.
        let webview: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
        let base = NSURL::URLWithString(&NSString::from_str(PAGE_URL));
        // SAFETY: plain Foundation arguments; called on the main thread by `with_webview`.
        unsafe {
            webview.loadHTMLString_baseURL(&NSString::from_str(&html), base.as_deref());
        }
    })
    .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn load_page(view: &tauri::Webview, _src: &str, html: String) -> CmdResult<()> {
    view.with_webview(move |platform| {
        use webkit2gtk::WebViewExt;
        platform.inner().load_html(&html, Some(PAGE_URL));
    })
    .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn load_page(view: &tauri::Webview, src: &str, _html: String) -> CmdResult<()> {
    let mut url = Url::parse(PAGE_URL).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("src", src);
    view.navigate(url).map_err(|e| e.to_string())
}

/// Serves the player page on Windows: `https://termlab-station.localhost/?src=<player>`.
#[cfg(windows)]
pub fn protocol<R: tauri::Runtime>(
    _ctx: tauri::UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let src = Url::parse(&request.uri().to_string())
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "src")
                .map(|(_, v)| v.into_owned())
        })
        .and_then(|s| player_src(&s).ok());
    let response = tauri::http::Response::builder();
    match src {
        Some(src) => response
            .header("Content-Type", "text/html; charset=utf-8")
            .body(page_html(&src).into_bytes()),
        None => response.status(404).body(Vec::new()),
    }
    .unwrap_or_default()
}

fn bounds(place: Option<[f64; 4]>) -> Rect {
    let [x, y, width, height] = place.unwrap_or([AWAY, AWAY, PLAYER_WIDTH, PLAYER_HEIGHT]);
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(width, height).into(),
    }
}

fn create(app: &AppHandle) -> CmdResult<tauri::Webview> {
    let window = app
        .get_window(MAIN_WINDOW)
        .ok_or("Termlab's window isn't open")?;
    let blank = "about:blank".parse().map_err(|e| format!("{e}"))?;
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(blank))
        // Links from the player (its title, the YouTube logo) open in the browser.
        .on_new_window(|url, _| {
            if matches!(url.scheme(), "https" | "http") {
                open_in_browser(url.as_str());
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_navigation(allowed_in_view);
    #[cfg(windows)]
    let builder = builder.use_https_scheme(true);
    let place = bounds(PLACE.lock().ok().and_then(|p| *p));
    window
        .add_child(builder, place.position, place.size)
        .map_err(|e| format!("couldn't open the player: {e}"))
}

/// Plays `src` (a YouTube player address) in the station view, creating it on first use.
#[tauri::command]
pub async fn station_open(app: AppHandle, src: String) -> CmdResult<()> {
    let src = player_src(&src)?;
    let view = match app.get_webview(LABEL) {
        Some(view) => view,
        None => {
            let view = create(&app)?;
            // A placement may have arrived while the view was being made.
            let place = PLACE.lock().ok().and_then(|p| *p);
            view.set_bounds(bounds(place)).map_err(|e| e.to_string())?;
            view
        }
    };
    let html = page_html(&src);
    load_page(&view, &src, html)
}

/// Places the view over `rect` of the main window (CSS pixels), or out of view for None. The
/// music keeps playing either way.
#[tauri::command]
pub async fn station_place(app: AppHandle, rect: Option<DockRect>) -> CmdResult<()> {
    let place = match rect {
        Some(rect) => {
            let ok = [rect.x, rect.y, rect.width, rect.height]
                .iter()
                .all(|v| v.is_finite())
                && rect.width >= 16.0
                && rect.height >= 16.0;
            if !ok {
                return Err("invalid player position".into());
            }
            // Page coordinates and the window's content view don't share an origin: see
            // `page_origin`.
            let (dx, dy) = page_origin(&app).await;
            Some([
                (rect.x + dx).max(0.0),
                (rect.y + dy).max(0.0),
                rect.width,
                rect.height,
            ])
        }
        None => None,
    };
    if let Ok(mut p) = PLACE.lock() {
        *p = place;
    }
    match app.get_webview(LABEL) {
        Some(view) => view.set_bounds(bounds(place)).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Sends a message to the player (its postMessage protocol, as JSON).
#[tauri::command]
pub async fn station_post(app: AppHandle, message: String) -> CmdResult<()> {
    let Some(view) = app.get_webview(LABEL) else {
        return Ok(());
    };
    let literal = serde_json::to_string(&message).map_err(|e| e.to_string())?;
    view.eval(format!(
        "window.__termlabStation && window.__termlabStation.post({literal})"
    ))
    .map_err(|e| e.to_string())
}

/// The player's messages since the last call, oldest first.
#[tauri::command]
pub async fn station_events(app: AppHandle) -> CmdResult<Vec<String>> {
    let Some(view) = app.get_webview(LABEL) else {
        return Ok(Vec::new());
    };
    let json = eval_json(
        &view,
        "window.__termlabStation ? window.__termlabStation.drain() : []",
    )
    .await;
    Ok(json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}

/// Stops the player: the view is emptied and taken out of view (kept for the next station;
/// closing it could race a station started right after).
#[tauri::command]
pub async fn station_stop(app: AppHandle) -> CmdResult<()> {
    if let Ok(mut p) = PLACE.lock() {
        *p = None;
    }
    let Some(view) = app.get_webview(LABEL) else {
        return Ok(());
    };
    view.navigate("about:blank".parse().map_err(|e| format!("{e}"))?)
        .map_err(|e| e.to_string())?;
    view.set_bounds(bounds(None)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_src_names_the_embedding_page() {
        let src = player_src(
            "https://www.youtube-nocookie.com/embed/rFZHOHl-L8A?autoplay=1&origin=tauri%3A%2F%2Flocalhost",
        )
        .unwrap();
        let url = Url::parse(&src).unwrap();
        let origins: Vec<_> = url
            .query_pairs()
            .filter(|(k, _)| k == "origin")
            .map(|(_, v)| v.into_owned())
            .collect();
        assert_eq!(origins, [PAGE_URL.trim_end_matches('/')]);
        assert!(url.query_pairs().any(|(k, v)| k == "autoplay" && v == "1"));
    }

    #[test]
    fn player_src_is_only_youtubes_player() {
        assert!(player_src("https://www.youtube-nocookie.com/embed/videoseries?list=PL1").is_ok());
        assert!(player_src("http://www.youtube-nocookie.com/embed/x").is_err());
        assert!(player_src("https://www.youtube.com/embed/x").is_err());
        assert!(player_src("https://www.youtube-nocookie.com/watch?v=x").is_err());
        assert!(player_src("https://www.youtube-nocookie.com.evil.example/embed/x").is_err());
        assert!(player_src("javascript:alert(1)").is_err());
    }

    #[test]
    fn page_quotes_the_address() {
        let html = page_html("https://www.youtube-nocookie.com/embed/x?a=1&b=\"><script>");
        assert!(html.contains(
            "src=\"https://www.youtube-nocookie.com/embed/x?a=1&amp;b=&quot;&gt;&lt;script&gt;\""
        ));
    }
}
