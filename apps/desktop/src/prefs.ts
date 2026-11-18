// Per-viewer preferences chosen in Settings (notifications, terminal), kept in localStorage.
// Changes apply at once: components read them through `usePrefs()` or listen for PREFS_EVENT.

import { useSyncExternalStore } from "react";

export type CursorStyle = "bar" | "block" | "underline";

export interface Prefs {
  /** A native notification when a session asks for you. */
  notifyNeedsYou: boolean;
  /** A native notification when a session finishes while Termlab is in the background. */
  notifyFinished: boolean;
  /** The dock / taskbar badge counting sessions that need you. */
  dockBadge: boolean;
  terminalFontSize: number;
  terminalLineHeight: number;
  cursorStyle: CursorStyle;
  cursorBlink: boolean;
  /** Lines per mouse-wheel / trackpad step, as a multiplier of xterm's default. */
  terminalScrollSpeed: number;
}

export const DEFAULT_PREFS: Prefs = {
  notifyNeedsYou: true,
  notifyFinished: true,
  dockBadge: true,
  terminalFontSize: 13,
  terminalLineHeight: 1.15,
  cursorStyle: "block",
  cursorBlink: true,
  terminalScrollSpeed: 1,
};

export const FONT_SIZE_RANGE = { min: 10, max: 20 } as const;
export const LINE_HEIGHTS = [
  { value: 1, label: "Compact" },
  { value: 1.15, label: "Normal" },
  { value: 1.3, label: "Roomy" },
] as const;

export const SCROLL_SPEEDS = [
  { value: 1, label: "Normal" },
  { value: 3, label: "Fast" },
  { value: 6, label: "Faster" },
] as const;

const KEY = "termlab.prefs";
export const PREFS_EVENT = "termlab-prefs";

function load(): Prefs {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Prefs>;
    return sanitize({ ...DEFAULT_PREFS, ...(raw && typeof raw === "object" ? raw : {}) });
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

function sanitize(p: Prefs): Prefs {
  const bool = (v: unknown, d: boolean) => (typeof v === "boolean" ? v : d);
  const size = Number(p.terminalFontSize);
  const lineHeight = Number(p.terminalLineHeight);
  const scrollSpeed = Number(p.terminalScrollSpeed);
  return {
    notifyNeedsYou: bool(p.notifyNeedsYou, DEFAULT_PREFS.notifyNeedsYou),
    notifyFinished: bool(p.notifyFinished, DEFAULT_PREFS.notifyFinished),
    dockBadge: bool(p.dockBadge, DEFAULT_PREFS.dockBadge),
    terminalFontSize: Number.isFinite(size)
      ? Math.min(FONT_SIZE_RANGE.max, Math.max(FONT_SIZE_RANGE.min, Math.round(size)))
      : DEFAULT_PREFS.terminalFontSize,
    terminalLineHeight: LINE_HEIGHTS.some((l) => l.value === lineHeight) ? lineHeight : DEFAULT_PREFS.terminalLineHeight,
    cursorStyle: (["bar", "block", "underline"] as const).includes(p.cursorStyle) ? p.cursorStyle : DEFAULT_PREFS.cursorStyle,
    cursorBlink: bool(p.cursorBlink, DEFAULT_PREFS.cursorBlink),
    terminalScrollSpeed: SCROLL_SPEEDS.some((s) => s.value === scrollSpeed) ? scrollSpeed : DEFAULT_PREFS.terminalScrollSpeed,
  };
}

let prefs: Prefs = load();

export function getPrefs(): Prefs {
  return prefs;
}

export function setPrefs(patch: Partial<Prefs>) {
  prefs = sanitize({ ...prefs, ...patch });
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Storage unavailable: the choice lasts for this window only.
  }
  window.dispatchEvent(new Event(PREFS_EVENT));
}

function subscribe(listener: () => void) {
  window.addEventListener(PREFS_EVENT, listener);
  return () => window.removeEventListener(PREFS_EVENT, listener);
}

export function usePrefs(): Prefs {
  return useSyncExternalStore(subscribe, getPrefs);
}
