import { getCurrentWebview } from "@tauri-apps/api/webview";

/**
 * Files dropped from Finder (or Explorer) onto a terminal type their paths into it, the way
 * Terminal.app and iTerm do. The web view never sees these drops as DOM events: Tauri takes
 * them over to hand out real file paths, so one listener here routes each drop to the
 * terminal under the pointer.
 */

type Target = { el: HTMLElement; onDrop: (paths: string[]) => void };

const targets = new Set<Target>();
let hovered: Target | null = null;
let installed = false;

const IS_WINDOWS = typeof navigator !== "undefined" && /Win/.test(navigator.platform || navigator.userAgent);

/** Registers a drop target; returns the function that removes it. */
export function onFileDrop(el: HTMLElement, onDrop: (paths: string[]) => void): () => void {
  const target = { el, onDrop };
  targets.add(target);
  install();
  return () => {
    targets.delete(target);
    if (hovered === target) setHovered(null);
  };
}

function install() {
  if (installed || !("__TAURI_INTERNALS__" in window)) return;
  installed = true;
  void getCurrentWebview()
    .onDragDropEvent(({ payload }) => {
      if (payload.type === "leave") return setHovered(null);
      const target = targetAt(payload.position);
      if (payload.type === "drop") {
        setHovered(null);
        if (target && payload.paths.length > 0) target.onDrop(payload.paths);
      } else {
        setHovered(target);
      }
    })
    .catch(() => {
      installed = false;
    });
}

function targetAt(position: { x: number; y: number }): Target | null {
  // WebView2 reports device pixels; WKWebView and WebKitGTK report CSS pixels already.
  const scale = IS_WINDOWS ? window.devicePixelRatio || 1 : 1;
  const x = position.x / scale;
  const y = position.y / scale;
  for (const t of targets) {
    const r = t.el.getBoundingClientRect();
    if (r.width > 0 && r.height > 0 && x >= r.left && x < r.right && y >= r.top && y < r.bottom) return t;
  }
  return null;
}

function setHovered(target: Target | null) {
  if (hovered === target) return;
  hovered?.el.classList.remove("is-drop-target");
  hovered = target;
  hovered?.el.classList.add("is-drop-target");
}

/** Paths as a shell reads them back, separated by spaces, with a trailing space like Terminal.app. */
export function quotePaths(paths: string[]): string {
  return paths.map(IS_WINDOWS ? quoteWindows : quotePosix).join(" ") + " ";
}

function quotePosix(path: string): string {
  return path.replace(/[^\w\-.,:/@+%=\u0080-￿]/g, (c) => (c === "\n" ? "'\n'" : `\\${c}`));
}

function quoteWindows(path: string): string {
  return /[\s&()^;,=!'+[\]{}%`~]/.test(path) ? `"${path}"` : path;
}
