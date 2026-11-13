//! YouTube Music docked in the sidebar: music.youtube.com in a child webview of the main
//! window, placed over a slot the frontend measures. (An iframe can't do this: YouTube Music
//! refuses to be framed.) The user signs in there and uses it as usual; the sidebar also
//! shows what's playing and has play/pause/next.
//!
//! The page is remote content, so it has no access to Termlab's commands (Tauri rejects IPC
//! from remote origins unless a capability grants it, and none does). Termlab only reads the
//! page's Media Session metadata and clicks its player buttons, via `eval`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Rect, WebviewUrl};

const LABEL: &str = "ytmusic";
const MAIN_WINDOW: &str = "main";
const HOME: &str = "https://music.youtube.com/";
/// The view identifies as Safari on an iPhone, for two reasons. YouTube Music picks its
/// layout from the browser's identity: a phone gets the mobile layout, which is made for a
/// column as narrow as the sidebar (a desktop browser gets the desktop layout at any width).
/// And Google refuses sign-in from embedded browsers it doesn't recognise; this view is
/// Safari's own engine (WKWebView), so presenting it as Safari is accurate enough for that.
const MOBILE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";

/// Runs in the docked page before its own scripts. Two adjustments, no access to Termlab:
///
/// - The page always counts as visible. YouTube Music's mobile site pauses when its page
///   goes to the background (the card collapsed, the window covered or minimised); as a
///   music player docked in a desktop app, it should keep playing, as the desktop site does.
/// - No scrollbars (scrolling still works): they'd take a fair share of a view this small.
///   `scrollbar-width` is the standard property, which current WebKit obeys over
///   `::-webkit-scrollbar` wherever a page sets the standard ones; both are set, in the page
///   and in every shadow root it creates.
const PAGE_SETUP_JS: &str = r#"(() => {
  try {
    const visible = { hidden: false, webkitHidden: false, visibilityState: "visible", webkitVisibilityState: "visible" };
    for (const [name, value] of Object.entries(visible)) {
      Object.defineProperty(Document.prototype, name, { configurable: true, get: () => value });
    }
    for (const type of ["visibilitychange", "webkitvisibilitychange"]) {
      window.addEventListener(type, (e) => e.stopImmediatePropagation(), true);
    }
  } catch (e) {}

  const css = "*{scrollbar-width:none!important}::-webkit-scrollbar{display:none!important;width:0!important;height:0!important}";
  try {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(css);
    const attach = Element.prototype.attachShadow;
    Element.prototype.attachShadow = function (init) {
      const root = attach.call(this, init);
      try { root.adoptedStyleSheets = [...root.adoptedStyleSheets, sheet]; } catch (e) {}
      return root;
    };
  } catch (e) {}
  const add = () => {
    if (document.getElementById("termlab-style")) return;
    const style = document.createElement("style");
    style.id = "termlab-style";
    style.textContent = css;
    (document.head || document.documentElement).appendChild(style);
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", add);
  else add();
})();"#;

type CmdResult<T> = Result<T, String>;

/// Hosts the view may navigate to (YouTube Music and Google sign-in). Anything else opens
/// in the default browser.
fn allowed_in_window(url: &tauri::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    let ok = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    ok("youtube.com")
        || ok("google.com")
        || ok("gstatic.com")
        || ok("googleusercontent.com")
        || ok("ytimg.com")
        || ok("youtube-nocookie.com")
        // Regional Google sign-in / consent hosts (google.nl, google.co.uk, …).
        || host.starts_with("accounts.google.")
        || host.starts_with("consent.google.")
        || host.starts_with("consent.youtube.")
}

/// A page on regular YouTube that isn't part of signing in. The view can end up there after
/// a Google sign-in or through a link, and it's meant to stay on YouTube Music.
fn strayed_to_youtube(url: &tauri::Url) -> bool {
    let on_youtube = matches!(
        url.host_str(),
        Some("www.youtube.com" | "youtube.com" | "m.youtube.com")
    );
    let signing_in = ["/signin", "/accounts", "/logout"]
        .iter()
        .any(|p| url.path().starts_with(p));
    on_youtube && !signing_in
}

/// Whether another automatic return to YouTube Music is allowed: a few per minute, so a page
/// that keeps redirecting away can't cause a loop.
fn may_return_home() -> bool {
    use std::time::Instant;
    static RECENT: std::sync::Mutex<Vec<Instant>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut recent) = RECENT.lock() else {
        return false;
    };
    let now = Instant::now();
    recent.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
    if recent.len() >= 3 {
        return false;
    }
    recent.push(now);
    true
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    #[cfg(not(target_os = "macos"))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// Where the frontend wants the page, in CSS pixels from the window's top-left corner.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct DockRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Shows YouTube Music over `rect` of the main window at `zoom` (below 1 shrinks the page to
/// fit the sidebar). The view is created on first use and then only moved, so music keeps
/// playing.
#[tauri::command]
pub async fn youtube_music_dock(app: AppHandle, rect: DockRect, zoom: f64) -> CmdResult<()> {
    let ok = [rect.x, rect.y, rect.width, rect.height, zoom]
        .iter()
        .all(|v| v.is_finite())
        && rect.width >= 40.0
        && rect.height >= 40.0;
    if !ok {
        return Err("invalid dock position".into());
    }
    // `rect` is in the page's CSS pixels, but the docked view is positioned in the window's
    // content view, and the two don't share an origin: see `page_origin`.
    let (dx, dy) = page_origin(&app).await;
    let position = LogicalPosition::new((rect.x + dx).max(0.0), (rect.y + dy).max(0.0));
    let size = LogicalSize::new(rect.width, rect.height);
    let view = match app.get_webview(LABEL) {
        Some(view) => {
            // Apply the complete rectangle at once. Separate position/size updates
            // read back native bounds between calls, which can shift the view on macOS.
            view.set_bounds(Rect {
                position: position.into(),
                size: size.into(),
            })
            .map_err(|e| e.to_string())?;
            view
        }
        None => {
            let window = app
                .get_window(MAIN_WINDOW)
                .ok_or("Termlab's window isn't open")?;
            let url = HOME.parse().map_err(|e| format!("{e}"))?;
            let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(url))
                .user_agent(MOBILE_UA)
                .initialization_script(PAGE_SETUP_JS)
                .on_page_load(|view, payload| {
                    if matches!(payload.event(), PageLoadEvent::Finished)
                        && strayed_to_youtube(payload.url())
                        && may_return_home()
                        && let Ok(home) = HOME.parse()
                    {
                        let _ = view.navigate(home);
                    }
                })
                .on_navigation(|url| {
                    if allowed_in_window(url) {
                        true
                    } else {
                        if matches!(url.scheme(), "https" | "http") {
                            open_in_browser(url.as_str());
                        }
                        false
                    }
                });
            window
                .add_child(builder, position, size)
                .map_err(|e| format!("couldn't open YouTube Music: {e}"))?
        }
    };
    view.set_zoom(zoom.clamp(0.3, 1.5))
        .map_err(|e| e.to_string())?;
    view.show().map_err(|e| e.to_string())
}

/// Where Termlab's page starts inside the window's content view, in logical pixels.
///
/// Termlab's own view spans the whole window, title bar included (with several views in a
/// window, Tauri sizes the main one to the full window), and macOS moves the *page* down by
/// the title bar's height so it isn't hidden behind it. So page coordinates are below view
/// coordinates by that inset, which is the difference between the view's height and the
/// page's viewport height. Measured each time: it's zero in fullscreen.
async fn page_origin(app: &AppHandle) -> (f64, f64) {
    static LAST_INSET: std::sync::Mutex<f64> = std::sync::Mutex::new(0.0);
    let last = || LAST_INSET.lock().map(|v| *v).unwrap_or(0.0);
    let Some(main) = app.get_webview(MAIN_WINDOW) else {
        return (0.0, last());
    };
    let scale = main.window().scale_factor().unwrap_or(1.0);
    let Ok(bounds) = main.bounds() else {
        return (0.0, last());
    };
    let origin = bounds.position.to_logical::<f64>(scale);
    let view_height = bounds.size.to_logical::<f64>(scale).height;
    let inset = match eval_json(&main, "window.innerHeight")
        .await
        .and_then(|v| v.parse::<f64>().ok())
    {
        Some(viewport_height) => top_inset(view_height, viewport_height),
        None => last(),
    };
    if let Ok(mut v) = LAST_INSET.lock() {
        *v = inset;
    }
    (origin.x, origin.y + inset)
}

/// The page's top inset: how much shorter its viewport is than its view. Anything implausible
/// (a zoomed page, a view mid-resize) counts as none.
fn top_inset(view_height: f64, viewport_height: f64) -> f64 {
    let inset = view_height - viewport_height;
    if (0.0..=120.0).contains(&inset) {
        inset
    } else {
        0.0
    }
}

/// Runs `js` in `view` and returns its result as JSON (None if it doesn't answer in time).
async fn eval_json(view: &tauri::Webview, js: &str) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = std::sync::Mutex::new(Some(tx));
    view.eval_with_callback(js, move |json| {
        if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
            let _ = tx.send(json);
        }
    })
    .ok()?;
    tokio::time::timeout(Duration::from_millis(400), rx)
        .await
        .ok()?
        .ok()
}

/// Sets the docked page's volume (0 to 100). Uses the page's own player when it has one, so
/// the level carries over to the next song, and sets the media directly as well.
#[tauri::command]
pub async fn youtube_music_volume(app: AppHandle, volume: f64) -> CmdResult<()> {
    let Some(view) = app.get_webview(LABEL) else {
        return Ok(());
    };
    if !volume.is_finite() {
        return Err("invalid volume".into());
    }
    let level = volume.clamp(0.0, 100.0).round() as u32;
    let js = format!(
        r#"(() => {{
  const n = {level};
  const p = document.getElementById("movie_player");
  if (p && typeof p.setVolume === "function") {{
    if (n > 0 && typeof p.unMute === "function") p.unMute();
    p.setVolume(n);
  }}
  document.querySelectorAll("video, audio").forEach((m) => {{ m.volume = n / 100; m.muted = n === 0; }});
}})()"#
    );
    view.eval(js).map_err(|e| e.to_string())
}

/// Moves the docked page: back one page, to YouTube Music's home, or reloads it.
#[tauri::command]
pub async fn youtube_music_navigate(app: AppHandle, action: String) -> CmdResult<()> {
    let Some(view) = app.get_webview(LABEL) else {
        return Ok(());
    };
    match action.as_str() {
        "home" => view
            .navigate(HOME.parse().map_err(|e| format!("{e}"))?)
            .map_err(|e| e.to_string()),
        "back" => view.eval("history.back()").map_err(|e| e.to_string()),
        "reload" => view.reload().map_err(|e| e.to_string()),
        other => Err(format!("unknown action {other}")),
    }
}

/// Takes the page out of view. It is moved outside the window rather than hidden: a hidden
/// view's page counts as being in the background, and it should carry on as it is.
#[tauri::command]
pub async fn youtube_music_hide(app: AppHandle) -> CmdResult<()> {
    match app.get_webview(LABEL) {
        Some(view) => view
            .set_position(LogicalPosition::new(-20_000.0, -20_000.0))
            .map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub artwork: Option<String>,
    pub playing: bool,
    /// The player's volume, 0 to 100 (0 when muted).
    #[serde(default)]
    pub volume: Option<f64>,
}

/// What the page's Media Session reports (the same data macOS shows in Now Playing).
const STATE_JS: &str = r#"(() => {
  const md = navigator.mediaSession && navigator.mediaSession.metadata;
  if (!md || !md.title) return null;
  const v = document.querySelector("video");
  const art = Array.from(md.artwork || []);
  // The page's player has its own volume (it re-applies it to every song).
  const p = document.getElementById("movie_player");
  let volume = null;
  if (p && typeof p.getVolume === "function") {
    volume = typeof p.isMuted === "function" && p.isMuted() ? 0 : Number(p.getVolume());
  } else if (v) {
    volume = v.muted ? 0 : Math.round(v.volume * 100);
  }
  return {
    title: String(md.title),
    artist: String(md.artist || ""),
    artwork: art.length ? String(art[art.length - 1].src) : null,
    playing: !!v && !v.paused,
    volume: Number.isFinite(volume) ? volume : null,
  };
})()"#;

/// What's playing in YouTube Music, or null when it hasn't been opened or nothing has played.
#[tauri::command]
pub async fn youtube_music_state(app: AppHandle) -> CmdResult<Option<NowPlaying>> {
    let Some(w) = app.get_webview(LABEL) else {
        return Ok(None);
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = std::sync::Mutex::new(Some(tx));
    w.eval_with_callback(STATE_JS, move |json| {
        if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
            let _ = tx.send(json);
        }
    })
    .map_err(|e| e.to_string())?;
    match tokio::time::timeout(Duration::from_secs(2), rx).await {
        Ok(Ok(json)) => Ok(serde_json::from_str(&json).ok().flatten()),
        _ => Ok(None),
    }
}

const PLAY_PAUSE_JS: &str = r##"(() => {
  const b = document.querySelector("ytmusic-player-bar #play-pause-button, #play-pause-button, .play-pause-button");
  if (b) return void b.click();
  const m = document.querySelector("video, audio");
  if (m) { if (m.paused) m.play(); else m.pause(); }
})()"##;

const SKIP_NEXT_JS: &str = r#"(() => {
  const b = document.querySelector(".next-button, [aria-label='Next'], [aria-label='Next song']");
  if (b) b.click();
  else document.dispatchEvent(new KeyboardEvent("keydown", { key: "k", code: "KeyK", keyCode: 75, which: 75, bubbles: true }));
})()"#;

const SKIP_PREVIOUS_JS: &str = r#"(() => {
  const b = document.querySelector(".previous-button, [aria-label='Previous'], [aria-label='Previous song']");
  if (b) b.click();
  else document.dispatchEvent(new KeyboardEvent("keydown", { key: "j", code: "KeyJ", keyCode: 74, which: 74, bubbles: true }));
})()"#;

/// Controls playback in the docked page.
#[tauri::command]
pub async fn youtube_music_control(app: AppHandle, action: String) -> CmdResult<()> {
    let Some(w) = app.get_webview(LABEL) else {
        return Ok(());
    };
    let js = match action.as_str() {
        // The page's own play/pause button (pressing it works in every layout, visible or
        // not); only without one, the media itself.
        "playPause" => PLAY_PAUSE_JS,
        "pause" => {
            r#"document.querySelectorAll("video, audio").forEach((m) => { if (!m.paused) m.pause(); })"#
        }
        // Next and previous: the page's own button when the layout has one, else its
        // keyboard shortcut (j: previous, k: next).
        "next" => SKIP_NEXT_JS,
        "previous" => SKIP_PREVIOUS_JS,
        other => return Err(format!("unknown action {other}")),
    };
    w.eval(js).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::allowed_in_window;

    fn ok(u: &str) -> bool {
        allowed_in_window(&u.parse().unwrap())
    }

    #[test]
    fn page_inset_is_the_title_bar() {
        use super::top_inset;
        // Measured on macOS 26: a 900 px view whose page is 868 px tall.
        assert_eq!(top_inset(900.0, 868.0), 32.0);
        // Fullscreen: no title bar.
        assert_eq!(top_inset(900.0, 900.0), 0.0);
        // Implausible values are ignored.
        assert_eq!(top_inset(900.0, 1200.0), 0.0);
        assert_eq!(top_inset(900.0, 300.0), 0.0);
    }

    #[test]
    fn notices_when_the_view_strays_to_regular_youtube() {
        let strayed = |u: &str| super::strayed_to_youtube(&u.parse().unwrap());
        assert!(strayed("https://www.youtube.com/"));
        assert!(strayed("https://www.youtube.com/watch?v=abc"));
        assert!(strayed("https://m.youtube.com/feed/subscriptions"));
        // YouTube Music itself, and the steps of signing in, are left alone.
        assert!(!strayed("https://music.youtube.com/watch?v=abc"));
        assert!(!strayed(
            "https://www.youtube.com/signin?action_handle_signin=true&next=x"
        ));
        assert!(!strayed("https://accounts.youtube.com/accounts/SetSID"));
        assert!(!strayed("https://accounts.google.com/v3/signin/identifier"));
    }

    #[test]
    fn stays_on_youtube_music_and_google_sign_in() {
        assert!(ok("https://music.youtube.com/watch?v=abc"));
        assert!(ok("https://accounts.google.com/v3/signin/identifier"));
        assert!(ok("https://accounts.google.nl/accounts/SetSID"));
        assert!(ok("https://consent.youtube.com/m?continue=x"));
        assert!(!ok("http://music.youtube.com/"));
        assert!(!ok("https://example.com/"));
        assert!(!ok("https://youtube.com.evil.example/"));
    }
}
