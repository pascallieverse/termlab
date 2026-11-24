import assert from "node:assert/strict";
import test from "node:test";
import {
  EMPTY_PANES,
  MAX_PANES,
  activate,
  closePane,
  dropAt,
  geometry,
  leaves,
  neighbour,
  parsePanes,
  place,
  reconcile,
  setRatio,
} from "../src/paneLayout.ts";

const row = (a, b, ratio = 0.5) => ({ dir: "row", ratio, a, b });
const col = (a, b, ratio = 0.5) => ({ dir: "col", ratio, a, b });
const SIZE = { w: 1000, h: 600 };

test("selecting a session replaces the active pane, or focuses the pane it is already in", () => {
  assert.deepEqual(activate(EMPTY_PANES, "a"), { layout: "a", active: "a" });
  assert.deepEqual(activate({ layout: "a", active: "a" }, "b"), { layout: "b", active: "b" });
  const two = { layout: row("a", "b"), active: "a" };
  assert.deepEqual(activate(two, "b"), { layout: row("a", "b"), active: "b" });
  assert.deepEqual(activate({ ...two, active: "b" }, "c"), { layout: row("a", "c"), active: "c" });
  assert.equal(activate(two, "a"), two);
});

test("a session dropped on an edge splits that pane", () => {
  const one = { layout: "a", active: "a" };
  assert.deepEqual(place(one, "b", { pane: "a", zone: "right" }), { layout: row("a", "b"), active: "b" });
  assert.deepEqual(place(one, "b", { pane: "a", zone: "left" }), { layout: row("b", "a"), active: "b" });
  assert.deepEqual(place(one, "b", { pane: "a", zone: "top" }), { layout: col("b", "a"), active: "b" });
  const two = { layout: row("a", "b"), active: "a" };
  assert.deepEqual(place(two, "c", { pane: "b", zone: "bottom" }).layout, row("a", col("b", "c")));
});

test("a session dropped in the middle takes the pane over; an open pane swaps", () => {
  const two = { layout: row("a", "b"), active: "a" };
  assert.deepEqual(place(two, "c", { pane: "b", zone: "center" }), { layout: row("a", "c"), active: "c" });
  assert.deepEqual(place(two, "a", { pane: "b", zone: "center" }), { layout: row("b", "a"), active: "a" });
});

test("an open pane dragged to another edge moves there", () => {
  const three = { layout: row("a", col("b", "c")), active: "a" };
  assert.deepEqual(place(three, "c", { pane: "a", zone: "left" }).layout, row(row("c", "a"), "b"));
  // Onto itself: nothing moves, it just becomes active.
  assert.deepEqual(place(three, "c", { pane: "c", zone: "left" }), { ...three, active: "c" });
});

test("the outer edge makes a pane across the whole side, with an equal share", () => {
  const stacked = { layout: col("a", "b"), active: "a" };
  assert.deepEqual(place(stacked, "c", { pane: null, zone: "right" }).layout, row(col("a", "b"), "c", 0.5));
  const columns = { layout: row("a", "b"), active: "a" };
  const next = place(columns, "c", { pane: null, zone: "right" }).layout;
  assert.ok(Math.abs(next.ratio - 2 / 3) < 1e-9);
  assert.deepEqual(leaves(next), ["a", "b", "c"]);
});

test("opening from a menu or shortcut shares the row out equally", () => {
  let p = { layout: "a", active: "a" };
  for (const id of ["b", "c", "d"]) p = place(p, id, { pane: p.active, zone: "right" }, true);
  assert.deepEqual(leaves(p.layout), ["a", "b", "c", "d"]);
  assert.deepEqual(geometry(p.layout, { x: 0, y: 0, w: 1000, h: 600 }).panes.map((r) => r.w), [250, 250, 250, 250]);
  // Only the row the new pane joined: a column next to it keeps its sizes.
  const mixed = { layout: row(col("a", "b", 0.3), "c", 0.7), active: "b" };
  const next = place(mixed, "d", { pane: "b", zone: "right" }, true).layout;
  assert.deepEqual(next, row(col("a", row("b", "d"), 0.3), "c", 0.7));
  const down = place(mixed, "d", { pane: "b", zone: "bottom" }, true).layout;
  assert.deepEqual(geometry(down, { x: 0, y: 0, w: 1000, h: 600 }).panes.map((r) => [r.id, r.h]), [["a", 200], ["b", 200], ["d", 200], ["c", 600]]);
});

test("no more panes than the limit; moving and replacing still work when full", () => {
  let p = { layout: "s0", active: "s0" };
  for (let i = 1; i < MAX_PANES; i++) p = place(p, `s${i}`, { pane: "s0", zone: "right" });
  assert.equal(leaves(p.layout).length, MAX_PANES);
  assert.equal(place(p, "extra", { pane: "s0", zone: "left" }), p);
  assert.ok(leaves(place(p, "extra", { pane: "s0", zone: "center" }).layout).includes("extra"));
  assert.equal(leaves(place(p, "s1", { pane: "s0", zone: "top" }).layout).length, MAX_PANES);
  assert.equal(dropAt(p.layout, SIZE, 500, 300, "extra").zone, "center");
  assert.equal(dropAt(p.layout, SIZE, 2, 300, "extra").zone, "center");
});

test("closing a pane gives its room to its neighbour, which becomes active", () => {
  const three = { layout: row("a", col("b", "c")), active: "c" };
  assert.deepEqual(closePane(three, "c"), { layout: row("a", "b"), active: "b" });
  assert.deepEqual(closePane(three, "a"), { layout: col("b", "c"), active: "c" });
  const one = { layout: "a", active: "a" };
  assert.equal(closePane(one, "a"), one);
});

test("sessions that went away leave the layout", () => {
  const three = { layout: row("a", col("b", "c")), active: "b" };
  const all = () => true;
  assert.equal(reconcile(three, all, "a"), three);
  assert.deepEqual(reconcile(three, (id) => id !== "b", "a"), { layout: row("a", "c"), active: "c" });
  assert.deepEqual(reconcile(three, (id) => id === "a", "x"), { layout: "a", active: "a" });
  assert.deepEqual(reconcile(three, () => false, "x"), { layout: "x", active: "x" });
  assert.deepEqual(reconcile(three, () => false, null), EMPTY_PANES);
  assert.equal(reconcile(EMPTY_PANES, all, null), EMPTY_PANES);
});

test("panes and dividers land on whole pixels and fill the area", () => {
  const { panes, dividers } = geometry(row("a", col("b", "c", 0.3), 1 / 3), { x: 0, y: 0, w: 1001, h: 601 });
  assert.deepEqual(panes, [
    { id: "a", x: 0, y: 0, w: 334, h: 601 },
    { id: "b", x: 334, y: 0, w: 667, h: 180 },
    { id: "c", x: 334, y: 180, w: 667, h: 421 },
  ]);
  assert.deepEqual(dividers.map((d) => [d.path, d.dir, d.at]), [["", "row", 334], ["b", "col", 180]]);
});

test("dividers move within limits", () => {
  const l = row("a", col("b", "c"));
  assert.equal(setRatio(l, "", 0.7).ratio, 0.7);
  assert.equal(setRatio(l, "b", 0.25).b.ratio, 0.25);
  assert.equal(setRatio(l, "", 0.99).ratio, 0.9);
  assert.equal(setRatio(l, "", 0.5), l);
});

test("drop zones: edges split, the middle replaces, your own pane does nothing", () => {
  assert.equal(dropAt("a", SIZE, 100, 300, "b").zone, "left");
  assert.equal(dropAt("a", SIZE, 900, 300, "b").zone, "right");
  assert.equal(dropAt("a", SIZE, 500, 50, "b").zone, "top");
  assert.equal(dropAt("a", SIZE, 500, 550, "b").zone, "bottom");
  assert.deepEqual(dropAt("a", SIZE, 500, 300, "b"), { pane: "a", zone: "center", rect: { x: 0, y: 0, w: 1000, h: 600 } });
  assert.deepEqual(dropAt("a", SIZE, 900, 300, "b").rect, { x: 500, y: 0, w: 500, h: 600 });
  assert.equal(dropAt("a", SIZE, 500, 300, "a"), null);
  assert.equal(dropAt("a", SIZE, -5, 300, "b"), null);

  const two = row("a", "b");
  assert.deepEqual(dropAt(two, SIZE, 5, 300, "c"), { pane: null, zone: "left", rect: { x: 0, y: 0, w: 1000 / 3, h: 600 } });
  assert.equal(dropAt(two, SIZE, 250, 300, "c").pane, "a");
  assert.equal(dropAt(two, SIZE, 450, 300, "c").zone, "right");
  assert.equal(dropAt(two, SIZE, 250, 300, "a"), null);
  assert.deepEqual(dropAt(two, SIZE, 750, 300, "a"), { pane: "b", zone: "center", rect: { x: 500, y: 0, w: 500, h: 600 } });
});

test("moving between panes follows what is next to what", () => {
  const l = row("a", col("b", "c"));
  assert.equal(neighbour(l, "a", "right"), "b");
  assert.equal(neighbour(l, "b", "left"), "a");
  assert.equal(neighbour(l, "b", "bottom"), "c");
  assert.equal(neighbour(l, "c", "left"), "a");
  assert.equal(neighbour(l, "a", "left"), null);
  assert.equal(neighbour(l, "c", "bottom"), null);
});

test("a saved layout is read back, and a broken one is ignored", () => {
  const saved = { layout: row("a", col("b", "c", 0.02)), active: "c" };
  assert.deepEqual(parsePanes(JSON.parse(JSON.stringify(saved))), { layout: row("a", col("b", "c", 0.1)), active: "c" });
  assert.equal(parsePanes({ layout: row("a", "b"), active: "gone" }).active, "a");
  assert.equal(parsePanes({ layout: row("a", "a"), active: "a" }), EMPTY_PANES);
  assert.equal(parsePanes({ layout: { dir: "diagonal", ratio: 0.5, a: "a", b: "b" } }), EMPTY_PANES);
  assert.equal(parsePanes("nonsense"), EMPTY_PANES);
  assert.equal(parsePanes(null), EMPTY_PANES);
});
