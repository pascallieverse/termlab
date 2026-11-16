import assert from "node:assert/strict";
import test from "node:test";
import {
  DEFAULT_BINDINGS,
  comboFromEvent,
  comboProblem,
  formatCombo,
  resolveBindings,
  shortcutForEvent,
} from "../src/keybindings.ts";

const press = (key, code, mods = {}) => ({
  key, code, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods,
});

test("defaults match on the platform's own modifier only", () => {
  assert.equal(shortcutForEvent(press("k", "KeyK", { metaKey: true }), DEFAULT_BINDINGS, true), "palette");
  assert.equal(shortcutForEvent(press("k", "KeyK", { ctrlKey: true }), DEFAULT_BINDINGS, false), "palette");
  // Ctrl+D on a Mac is the terminal's EOF, not "split".
  assert.equal(shortcutForEvent(press("d", "KeyD", { ctrlKey: true }), DEFAULT_BINDINGS, true), null);
  assert.equal(shortcutForEvent(press("H", "KeyH", { metaKey: true, shiftKey: true }), DEFAULT_BINDINGS, true), "handOff");
  assert.equal(shortcutForEvent(press("n", "KeyN", { metaKey: true, shiftKey: true }), DEFAULT_BINDINGS, true), null);
});

test("shift and option don't change which key was pressed", () => {
  assert.equal(comboFromEvent(press("<", "Comma", { metaKey: true, shiftKey: true }), true).key, ",");
  assert.equal(comboFromEvent(press("˚", "KeyK", { metaKey: true, altKey: true }), true).key, "k");
  assert.equal(comboFromEvent(press("!", "Digit1", { ctrlKey: true, shiftKey: true }), false).key, "1");
  assert.equal(comboFromEvent(press("Shift", "ShiftLeft", { shiftKey: true }), true), null);
});

test("formats per platform", () => {
  const c = { mod: true, alt: true, shift: true, key: "k" };
  assert.equal(formatCombo(c, true), "⌥⇧⌘K");
  assert.equal(formatCombo(c, false), "Ctrl+Alt+Shift+K");
  assert.equal(formatCombo({ mod: false, alt: false, shift: false, key: "F5" }, false), "F5");
});

test("rejects combos that would take keys from the terminal or the system", () => {
  assert.match(comboProblem({ mod: false, alt: false, shift: false, key: "k" }, true), /Include ⌘/);
  assert.equal(comboProblem({ mod: false, alt: false, shift: false, key: "F5" }, true), null);
  assert.match(comboProblem({ mod: true, alt: false, shift: false, key: "c" }, true), /Copy/);
  assert.match(comboProblem({ mod: true, alt: false, shift: false, key: "q" }, true), /quitting/);
  assert.equal(comboProblem({ mod: true, alt: false, shift: false, key: "q" }, false), null);
  assert.equal(comboProblem({ mod: true, alt: false, shift: true, key: "c" }, true), null);
});

test("saved overrides: turned off, changed, malformed and duplicated", () => {
  const b = resolveBindings(
    {
      palette: { mod: true, alt: false, shift: true, key: "p" },
      inspector: null,
      settings: { mod: "yes", key: "," },
      newSession: { mod: true, alt: false, shift: true, key: "p" },
      split: { mod: true, alt: false, shift: false, key: "v" },
    },
    true,
  );
  assert.deepEqual(b.palette, { mod: true, alt: false, shift: true, key: "p" });
  assert.equal(b.inspector, null);
  assert.deepEqual(b.settings, DEFAULT_BINDINGS.settings);
  assert.equal(b.newSession, null, "the earlier action keeps a shared combo");
  assert.deepEqual(b.split, DEFAULT_BINDINGS.split, "a reserved combo falls back to the default");
  assert.deepEqual(resolveBindings("garbage", true), DEFAULT_BINDINGS);
});
