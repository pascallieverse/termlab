import assert from "node:assert/strict";
import test from "node:test";
import { arrowKeys, sgrWheelReports, wheelSteps } from "../src/terminalWheel.ts";

const PIXEL = 0;
const LINE = 1;
const wheel = (o = {}) => ({ deltaY: 0, deltaMode: PIXEL, altKey: false, ctrlKey: false, shiftKey: false, ...o });

test("a mouse-wheel notch makes `speed` steps", () => {
  for (const speed of [1, 3, 6]) {
    assert.equal(wheelSteps(wheel({ deltaY: -100 }), 17, speed, { value: 0 }), -speed);
    assert.equal(wheelSteps(wheel({ deltaY: 3, deltaMode: LINE }), 17, speed, { value: 0 }), speed);
  }
});

test("small trackpad movements add up before making a step", () => {
  const carry = { value: 0 };
  const steps = [];
  for (let i = 0; i < 20; i++) steps.push(wheelSteps(wheel({ deltaY: 10 }), 17, 3, carry));
  // 10px × 0.3 / 17px ≈ 0.18 lines each: a step every sixth event, three reports at a time.
  assert.deepEqual([...new Set(steps)].sort(), [0, 3]);
  assert.equal(steps.filter((s) => s === 3).length, 3);
});

test("Option or Ctrl scrolls five times faster; Shift and no movement don't scroll", () => {
  assert.equal(wheelSteps(wheel({ deltaY: 100, altKey: true }), 17, 3, { value: 0 }), 15);
  assert.equal(wheelSteps(wheel({ deltaY: 100, ctrlKey: true }), 17, 1, { value: 0 }), 5);
  assert.equal(wheelSteps(wheel({ deltaY: 100, shiftKey: true }), 17, 3, { value: 0 }), 0);
  assert.equal(wheelSteps(wheel({ deltaY: 0 }), 17, 3, { value: 0 }), 0);
});

test("SGR wheel reports carry the cell and xterm's modifier bits", () => {
  assert.equal(sgrWheelReports(-2, 5, 7, wheel()), "\x1b[<64;5;7M\x1b[<64;5;7M");
  assert.equal(sgrWheelReports(1, 1, 1, wheel({ altKey: true, ctrlKey: true })), "\x1b[<89;1;1M");
});

test("without mouse reporting the wheel sends cursor keys", () => {
  assert.equal(arrowKeys(-2, false), "\x1b[A\x1b[A");
  assert.equal(arrowKeys(3, true), "\x1bOB\x1bOB\x1bOB");
});
