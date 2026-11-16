// Keyboard shortcuts: the actions that can be rebound, their defaults, and how a key press
// becomes a combo. Pure functions, so they can be tested without a window.
//
// "Mod" is ⌘ on macOS and Ctrl elsewhere. On macOS a shortcut never fires on Ctrl, so
// Ctrl+D, Ctrl+K and friends keep reaching the terminal.

export interface Combo {
  mod: boolean;
  alt: boolean;
  shift: boolean;
  /** A lower-case letter or digit, a punctuation character, or a key name ("F5", "Enter"). */
  key: string;
}

export type ShortcutId =
  | "palette"
  | "newSession"
  | "settings"
  | "focusView"
  | "missionView"
  | "inspector"
  | "split"
  | "handOff";

export interface ShortcutDef {
  id: ShortcutId;
  label: string;
  group: "Anywhere" | "Selected session";
  default: Combo;
}

const mod = (key: string, shift = false): Combo => ({ mod: true, alt: false, shift, key });

export const SHORTCUTS: ShortcutDef[] = [
  { id: "palette", label: "Search or run a command", group: "Anywhere", default: mod("k") },
  { id: "newSession", label: "New session", group: "Anywhere", default: mod("n") },
  { id: "settings", label: "Settings", group: "Anywhere", default: mod(",") },
  { id: "focusView", label: "Focus view", group: "Anywhere", default: mod("1") },
  { id: "missionView", label: "Mission control", group: "Anywhere", default: mod("2") },
  { id: "inspector", label: "Show or hide the inspector", group: "Anywhere", default: mod("i") },
  { id: "split", label: "Split with another session, or close the split", group: "Selected session", default: mod("d") },
  { id: "handOff", label: "Hand off to another agent", group: "Selected session", default: mod("h", true) },
];

export type Bindings = Record<ShortcutId, Combo | null>;

export const DEFAULT_BINDINGS = Object.fromEntries(SHORTCUTS.map((s) => [s.id, s.default])) as Bindings;

/** The parts of a KeyboardEvent this file reads. */
export interface KeyLike {
  key: string;
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

const MODIFIER_KEYS = new Set(["Meta", "Control", "Alt", "AltGraph", "Shift", "CapsLock", "Fn", "FnLock", "OS", "Hyper", "Super"]);

// Shift and Option change what `key` reports (",", "<", "≤"), so punctuation is read from
// the physical key instead: the same key always records and matches the same way.
const PUNCTUATION: Record<string, string> = {
  Comma: ",", Period: ".", Slash: "/", Semicolon: ";", Quote: "'", BracketLeft: "[",
  BracketRight: "]", Backslash: "\\", Minus: "-", Equal: "=", Backquote: "`",
};

/** The key a press is about, normalised; null for a lone modifier. */
export function eventKey(e: KeyLike): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  // Letters follow the layout (AZERTY's A is "a"); Option on a Mac turns them into symbols,
  // so fall back to the physical key then.
  if (/^[a-z]$/i.test(e.key)) return e.key.toLowerCase();
  const letter = /^Key([A-Z])$/.exec(e.code);
  if (letter) return letter[1].toLowerCase();
  const digit = /^(?:Digit|Numpad)([0-9])$/.exec(e.code);
  if (digit) return digit[1];
  if (PUNCTUATION[e.code]) return PUNCTUATION[e.code];
  if (e.key === " " || e.code === "Space") return "Space";
  if (e.key.length === 1) return e.key.toLowerCase();
  return e.key || null;
}

/** The combo a press makes, or null when only modifiers are down. */
export function comboFromEvent(e: KeyLike, isMac: boolean): Combo | null {
  const key = eventKey(e);
  if (!key) return null;
  return { mod: isMac ? e.metaKey : e.ctrlKey, alt: e.altKey, shift: e.shiftKey, key };
}

export function sameCombo(a: Combo | null | undefined, b: Combo | null | undefined): boolean {
  return !!a && !!b && a.mod === b.mod && a.alt === b.alt && a.shift === b.shift && a.key === b.key;
}

/** Whether a press is this combo. The other platform's modifier (Ctrl on a Mac) must be up. */
export function matchesCombo(e: KeyLike, combo: Combo, isMac: boolean): boolean {
  const other = isMac ? e.ctrlKey : e.metaKey;
  if (other) return false;
  return sameCombo(comboFromEvent(e, isMac), combo);
}

const KEY_NAMES: Record<string, [mac: string, other: string]> = {
  ArrowUp: ["↑", "Up"], ArrowDown: ["↓", "Down"], ArrowLeft: ["←", "Left"], ArrowRight: ["→", "Right"],
  Enter: ["↵", "Enter"], Escape: ["Esc", "Esc"], Tab: ["⇥", "Tab"], Backspace: ["⌫", "Backspace"],
  Delete: ["⌦", "Delete"], Space: ["Space", "Space"], PageUp: ["PgUp", "PgUp"], PageDown: ["PgDn", "PgDn"],
  Home: ["Home", "Home"], End: ["End", "End"],
};

export function keyName(key: string, isMac: boolean): string {
  const named = KEY_NAMES[key];
  if (named) return isMac ? named[0] : named[1];
  return key.length === 1 ? key.toUpperCase() : key;
}

/** "⌥⇧⌘K" on a Mac, "Ctrl+Alt+Shift+K" elsewhere. */
export function formatCombo(c: Combo, isMac: boolean): string {
  const key = keyName(c.key, isMac);
  if (isMac) return `${c.alt ? "⌥" : ""}${c.shift ? "⇧" : ""}${c.mod ? "⌘" : ""}${key}`;
  return [c.mod && "Ctrl", c.alt && "Alt", c.shift && "Shift", key].filter(Boolean).join("+");
}

const FUNCTION_KEY = /^F([1-9]|1[0-9]|2[0-4])$/;

/** Why a combo can't be used, or null when it can. */
export function comboProblem(c: Combo, isMac: boolean): string | null {
  const modName = isMac ? "⌘" : "Ctrl";
  if (!c.mod && !FUNCTION_KEY.test(c.key)) {
    return `Include ${modName} (or use a function key), so the shortcut doesn't take a key from the terminal.`;
  }
  if (c.mod && !c.alt) {
    const editing: Record<string, string> = { c: "Copy", v: "Paste", x: "Cut", a: "Select all", z: c.shift ? "Redo" : "Undo" };
    if (editing[c.key] && (!c.shift || c.key === "z")) return `${formatCombo(c, isMac)} is kept for ${editing[c.key]}.`;
    if (isMac && !c.shift) {
      const system: Record<string, string> = { q: "quitting Termlab", w: "closing the window", h: "hiding Termlab", m: "minimising the window" };
      if (system[c.key]) return `${formatCombo(c, isMac)} is kept for ${system[c.key]}.`;
    }
  }
  return null;
}

function isCombo(v: unknown): v is Combo {
  if (!v || typeof v !== "object") return false;
  const c = v as Record<string, unknown>;
  return (
    typeof c.key === "string" && c.key.length > 0 && c.key.length <= 16 &&
    typeof c.mod === "boolean" && typeof c.alt === "boolean" && typeof c.shift === "boolean"
  );
}

/**
 * Saved overrides on top of the defaults. An override is a combo or null (turned off);
 * anything malformed falls back to the default. When two actions end up on one combo,
 * the first in SHORTCUTS keeps it.
 */
export function resolveBindings(saved: unknown, isMac: boolean): Bindings {
  const raw = saved && typeof saved === "object" ? (saved as Record<string, unknown>) : {};
  const out = { ...DEFAULT_BINDINGS };
  for (const s of SHORTCUTS) {
    if (!(s.id in raw)) continue;
    const v = raw[s.id];
    if (v === null) out[s.id] = null;
    else if (isCombo(v) && !comboProblem(v, isMac)) out[s.id] = { mod: v.mod, alt: v.alt, shift: v.shift, key: v.key };
  }
  const taken: Combo[] = [];
  for (const s of SHORTCUTS) {
    const c = out[s.id];
    if (!c) continue;
    if (taken.some((t) => sameCombo(t, c))) out[s.id] = null;
    else taken.push(c);
  }
  return out;
}

/** The action a press triggers, if any. */
export function shortcutForEvent(e: KeyLike, bindings: Bindings, isMac: boolean): ShortcutId | null {
  for (const s of SHORTCUTS) {
    const c = bindings[s.id];
    if (c && matchesCombo(e, c, isMac)) return s.id;
  }
  return null;
}
