import assert from "node:assert/strict";
import test from "node:test";
import { claudeNewlineHandler } from "../src/terminalKeys.ts";

function key(overrides = {}) {
  return {
    type: "keydown", key: "Enter", shiftKey: true,
    altKey: false, ctrlKey: false, metaKey: false, isComposing: false, keyCode: 13,
    prevented: false, stopped: false,
    preventDefault() { this.prevented = true; },
    stopPropagation() { this.stopped = true; },
    ...overrides,
  };
}

test("Shift+Enter sends one newline sequence and suppresses the submit path", () => {
  const writes = [];
  const handler = claudeNewlineHandler(() => "claude", (data) => writes.push(data));
  for (const type of ["keydown", "keypress", "keyup"]) {
    const event = key({ type });
    assert.equal(handler(event), false);
    assert.equal(event.prevented, true);
    assert.equal(event.stopped, true);
  }
  assert.deepEqual(writes, ["\x1b\r"]);
});

test("plain Enter, other shortcuts, and IME confirmation retain xterm handling", () => {
  const handler = claudeNewlineHandler(() => "claude", () => assert.fail("unexpected write"));
  for (const overrides of [
    { shiftKey: false }, { key: "a" }, { ctrlKey: true }, { altKey: true },
    { metaKey: true }, { isComposing: true }, { keyCode: 229 },
  ]) {
    const event = key(overrides);
    assert.equal(handler(event), true);
    assert.equal(event.prevented, false);
  }
});

test("agent changes are read at key time, without changing other agents", () => {
  let agent = "codex";
  const writes = [];
  const handler = claudeNewlineHandler(() => agent, (data) => writes.push(data));
  for (agent of ["codex", "vibe", "antigravity"]) assert.equal(handler(key()), true);
  assert.deepEqual(writes, []);
  agent = "claude";
  assert.equal(handler(key()), false);
  assert.deepEqual(writes, ["\x1b\r"]);
});
