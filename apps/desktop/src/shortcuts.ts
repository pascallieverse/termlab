// The keyboard shortcuts in use: defaults plus what was changed in Settings → Shortcuts,
// kept per viewer in localStorage. Labels everywhere read from here, so they follow changes.

import { useSyncExternalStore } from "react";
import {
  DEFAULT_BINDINGS,
  SHORTCUTS,
  formatCombo,
  resolveBindings,
  sameCombo,
  type Bindings,
  type Combo,
  type ShortcutId,
} from "./keybindings";

export const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

const KEY = "termlab.shortcuts";
const EVENT = "termlab-shortcuts";

function loadSaved(): Partial<Record<ShortcutId, Combo | null>> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}

let current: Bindings = resolveBindings(loadSaved(), IS_MAC);

/** Saves only what differs from the defaults, so a later change to a default still reaches you. */
function save(next: Bindings) {
  current = resolveBindings(next, IS_MAC);
  const overrides: Partial<Record<ShortcutId, Combo | null>> = {};
  for (const s of SHORTCUTS) {
    const c = current[s.id];
    if (!sameCombo(c, DEFAULT_BINDINGS[s.id]) && !(c === null && DEFAULT_BINDINGS[s.id] === null)) overrides[s.id] = c;
  }
  try {
    if (Object.keys(overrides).length) localStorage.setItem(KEY, JSON.stringify(overrides));
    else localStorage.removeItem(KEY);
  } catch {
    // Storage unavailable: the change lasts for this window only.
  }
  window.dispatchEvent(new Event(EVENT));
}

export function getBindings(): Bindings {
  return current;
}

export function useBindings(): Bindings {
  return useSyncExternalStore(
    (l) => {
      window.addEventListener(EVENT, l);
      return () => window.removeEventListener(EVENT, l);
    },
    getBindings,
  );
}

/** Sets a shortcut (null turns it off). Returns the action that had this combo and lost it. */
export function setBinding(id: ShortcutId, combo: Combo | null): ShortcutId | null {
  const next = { ...current, [id]: combo };
  let displaced: ShortcutId | null = null;
  if (combo) {
    for (const s of SHORTCUTS) {
      if (s.id !== id && sameCombo(next[s.id], combo)) {
        next[s.id] = null;
        displaced = s.id;
      }
    }
  }
  save(next);
  return displaced;
}

/** Back to the default; an action already using that combo gives it up. */
export function resetBinding(id: ShortcutId): ShortcutId | null {
  return setBinding(id, DEFAULT_BINDINGS[id]);
}

export function resetAllBindings() {
  save({ ...DEFAULT_BINDINGS });
}

export function isDefaultBinding(id: ShortcutId, b: Bindings = current): boolean {
  return sameCombo(b[id], DEFAULT_BINDINGS[id]) || (b[id] === null && DEFAULT_BINDINGS[id] === null);
}

/** "⌘K", or "" when the action has no shortcut. */
export function shortcutLabel(id: ShortcutId, b: Bindings = current): string {
  const c = b[id];
  return c ? formatCombo(c, IS_MAC) : "";
}

/** " (⌘K)" for a tooltip, or "" when the action has no shortcut. */
export function shortcutHint(id: ShortcutId, b: Bindings = current): string {
  const l = shortcutLabel(id, b);
  return l ? ` (${l})` : "";
}

// While Settings is recording a new shortcut, the app's shortcuts stand down.
let recording = false;
export function setRecordingShortcut(on: boolean) {
  recording = on;
}
export function isRecordingShortcut(): boolean {
  return recording;
}

/** The ⌘ / Ctrl label for fixed shortcuts (⌘↵ in dialogs). */
export const MOD_LABEL = IS_MAC ? "⌘" : "Ctrl+";
