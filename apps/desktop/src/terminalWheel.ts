// Scroll speed for agents that draw full-screen (Claude Code does): they turn on mouse
// reporting, so the wheel no longer scrolls xterm's own history but is reported to the agent,
// and xterm sends one report per wheel event however far the wheel moved, so its
// `scrollSensitivity` has no effect there. This counts wheel events the way xterm does and
// sends the chosen speed's number of reports for each one instead.

/** The parts of a WheelEvent the count depends on (plain objects in tests). */
export interface WheelLike {
  deltaY: number;
  deltaMode: number;
  altKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
}

const DOM_DELTA_PIXEL = 0;

/** Holding Option (Alt) or Ctrl multiplies the speed by this, like xterm's fast scroll. */
export const FAST_SCROLL = 5;

/**
 * How many wheel steps one event makes: 0 while a trackpad's small movements add up to a
 * line, otherwise `speed` (times FAST_SCROLL with Option / Ctrl held). Negative scrolls up.
 * `carry` keeps the part of a line left over between events, as xterm does.
 */
export function wheelSteps(
  ev: WheelLike,
  cellHeight: number,
  speed: number,
  carry: { value: number },
): number {
  // Shift+wheel is horizontal scrolling, which xterm doesn't report either.
  if (ev.deltaY === 0 || ev.shiftKey) return 0;
  if (ev.deltaMode === DOM_DELTA_PIXEL) {
    let lines = ev.deltaY / Math.max(1, cellHeight);
    if (Math.abs(ev.deltaY) < 50) lines *= 0.3;
    carry.value += lines;
    const whole = Math.trunc(carry.value);
    carry.value -= whole;
    if (whole === 0) return 0;
  }
  const fast = ev.altKey || ev.ctrlKey ? FAST_SCROLL : 1;
  return Math.sign(ev.deltaY) * speed * fast;
}

/** `count` SGR (mode 1006) wheel reports at a 1-based cell, with xterm's modifier bits. */
export function sgrWheelReports(steps: number, col: number, row: number, ev: WheelLike): string {
  const button = (steps < 0 ? 64 : 65) + (ev.altKey ? 8 : 0) + (ev.ctrlKey ? 16 : 0);
  return `\x1b[<${button};${col};${row}M`.repeat(Math.abs(steps));
}

/** `count` cursor-key presses, what xterm sends for the wheel on a full screen without mouse reporting. */
export function arrowKeys(steps: number, applicationCursorKeys: boolean): string {
  const key = `\x1b${applicationCursorKeys ? "O" : "["}${steps < 0 ? "A" : "B"}`;
  return key.repeat(Math.abs(steps));
}
