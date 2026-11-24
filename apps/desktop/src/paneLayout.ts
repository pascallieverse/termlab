// The focus view's panes: a tree of splits whose leaves are session ids, plus which pane is
// active. Pure functions, so they can be tested without a window.

export type Edge = "left" | "right" | "top" | "bottom";
export type Zone = Edge | "center";
/** "row": side by side. "col": one above the other. */
export type Dir = "row" | "col";

export interface Split {
  dir: Dir;
  /** The first child's share of the split, between 0 and 1. */
  ratio: number;
  a: Layout;
  b: Layout;
}

/** A session id, or two layouts next to each other. */
export type Layout = string | Split;

export interface Panes {
  layout: Layout | null;
  /** The pane that has the keyboard, the inspector and the shortcuts. */
  active: string | null;
}

/** Where a session lands: on a pane (`pane: null` is the whole area), at an edge or in place of it. */
export interface DropTarget {
  pane: string | null;
  zone: Zone;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PaneRect extends Rect {
  id: string;
}

export interface Divider {
  /** Which split it belongs to: the "a" / "b" steps down from the root. */
  path: string;
  dir: Dir;
  ratio: number;
  /** The split's own area. */
  area: Rect;
  /** Where the line is: an x for "row", a y for "col". */
  at: number;
}

export const EMPTY_PANES: Panes = { layout: null, active: null };
/** More than this and no terminal is wide enough to work in. */
export const MAX_PANES = 6;
export const MIN_RATIO = 0.1;

/** How far into a pane (as a share of its size) a drop still means "split at this edge". */
const EDGE_ZONE = 0.3;
/** Pixels along the outer edge of the whole area that mean "a pane across this whole side". */
const OUTER_BAND = 22;

const dirOf = (edge: Edge): Dir => (edge === "left" || edge === "right" ? "row" : "col");
const clampRatio = (r: number) => (Number.isFinite(r) ? Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, r)) : 0.5);

/** The session ids on screen, left to right and top to bottom. */
export function leaves(l: Layout | null): string[] {
  if (l === null) return [];
  return typeof l === "string" ? [l] : [...leaves(l.a), ...leaves(l.b)];
}

export function hasLeaf(l: Layout | null, id: string): boolean {
  if (l === null) return false;
  return typeof l === "string" ? l === id : hasLeaf(l.a, id) || hasLeaf(l.b, id);
}

/** Replaces leaves; a leaf mapped to null goes away and its neighbour takes the room. */
function mapLeaves(l: Layout, fn: (id: string) => Layout | null): Layout | null {
  if (typeof l === "string") return fn(l);
  const a = mapLeaves(l.a, fn);
  const b = mapLeaves(l.b, fn);
  if (a === l.a && b === l.b) return l;
  if (a === null) return b;
  if (b === null) return a;
  return { ...l, a, b };
}

function splitOf(existing: Layout, id: string, edge: Edge, share = 0.5): Split {
  const first = edge === "left" || edge === "top";
  return first
    ? { dir: dirOf(edge), ratio: clampRatio(share), a: id, b: existing }
    : { dir: dirOf(edge), ratio: clampRatio(1 - share), a: existing, b: id };
}

/** How many panes sit next to each other along a direction, at most. */
function span(l: Layout, dir: Dir): number {
  if (typeof l === "string") return 1;
  return l.dir === dir ? span(l.a, dir) + span(l.b, dir) : Math.max(span(l.a, dir), span(l.b, dir));
}

/** The pane that takes over when `id` closes: the nearest one in the split it shared. */
function neighbourOf(l: Layout, id: string): string | null {
  if (typeof l === "string") return null;
  if (l.a === id) return leaves(l.b)[0] ?? null;
  if (l.b === id) return leaves(l.a).at(-1) ?? null;
  return neighbourOf(l.a, id) ?? neighbourOf(l.b, id);
}

/**
 * Shows a session and makes it active. One that's already on screen keeps its pane;
 * otherwise it takes the active pane's place.
 */
export function activate(p: Panes, id: string | null): Panes {
  if (id === null) return p.layout === null && p.active === null ? p : EMPTY_PANES;
  if (p.layout === null) return { layout: id, active: id };
  if (hasLeaf(p.layout, id)) return p.active === id ? p : { layout: p.layout, active: id };
  const from = p.active !== null && hasLeaf(p.layout, p.active) ? p.active : leaves(p.layout)[0];
  return { layout: mapLeaves(p.layout, (x) => (x === from ? id : x)), active: id };
}

/** Whether the splits from `l` down to the pane `id` all run along `dir`. */
function lineReaches(l: Layout, id: string, dir: Dir): boolean {
  if (typeof l === "string") return l === id;
  return l.dir === dir && (lineReaches(l.a, id, dir) || lineReaches(l.b, id, dir));
}

function evenShares(l: Layout, dir: Dir): Layout {
  if (typeof l === "string" || l.dir !== dir) return l;
  const a = evenShares(l.a, dir);
  const b = evenShares(l.b, dir);
  const count = (x: Layout): number => (typeof x === "string" || x.dir !== dir ? 1 : count(x.a) + count(x.b));
  return { ...l, ratio: clampRatio(count(a) / (count(a) + count(b))), a, b };
}

/** Gives the panes lined up with `id` along `dir` (its row, or its column) equal room. */
export function evenOut(l: Layout, id: string, dir: Dir): Layout {
  if (typeof l === "string" || !hasLeaf(l, id)) return l;
  if (lineReaches(l, id, dir)) return evenShares(l, dir);
  return { ...l, a: evenOut(l.a, id, dir), b: evenOut(l.b, id, dir) };
}

/**
 * Puts a session where it was dropped and makes it active. A pane that's already open moves.
 * A drop halves the pane it lands on, as its highlight shows; `even` instead shares the row
 * or column out equally (opening from a menu or a shortcut, where nothing was aimed at).
 */
export function place(p: Panes, id: string, target: DropTarget, even = false): Panes {
  const layout = p.layout;
  if (layout === null) return { layout: id, active: id };
  const moving = hasLeaf(layout, id);
  if (target.pane === id) return activate(p, id);

  if (target.zone === "center") {
    const onto = target.pane;
    if (onto === null || !hasLeaf(layout, onto)) return p;
    // Two open panes trade places; a new session takes the pane over.
    const next = mapLeaves(layout, (x) => (x === onto ? id : x === id ? onto : x));
    return next === null ? p : { layout: next, active: id };
  }

  if (!moving && leaves(layout).length >= MAX_PANES) return p;
  const rest = moving ? mapLeaves(layout, (x) => (x === id ? null : x)) : layout;
  if (rest === null) return p;
  const edge = target.zone;
  if (target.pane === null) {
    // Across the whole side: an equal share next to the panes already along it.
    return { layout: splitOf(rest, id, edge, 1 / (span(rest, dirOf(edge)) + 1)), active: id };
  }
  const beside = target.pane;
  if (!hasLeaf(rest, beside)) return p;
  const next = mapLeaves(rest, (x) => (x === beside ? splitOf(x, id, edge) : x));
  return next === null ? p : { layout: even ? evenOut(next, id, dirOf(edge)) : next, active: id };
}

/** Closes a pane; the last one stays. */
export function closePane(p: Panes, id: string): Panes {
  if (p.layout === null || !hasLeaf(p.layout, id)) return p;
  const next = mapLeaves(p.layout, (x) => (x === id ? null : x));
  if (next === null) return p;
  const active = p.active !== null && p.active !== id && hasLeaf(next, p.active) ? p.active : (neighbourOf(p.layout, id) ?? leaves(next)[0]);
  return { layout: next, active };
}

/**
 * Drops panes whose session is gone and keeps an active pane. With nothing left, `fallback`
 * (if any) is shown. Returns the same object when nothing changes.
 */
export function reconcile(p: Panes, exists: (id: string) => boolean, fallback: string | null): Panes {
  const layout = p.layout === null ? null : mapLeaves(p.layout, (x) => (exists(x) ? x : null));
  if (layout === null) {
    if (fallback === null) return p.layout === null && p.active === null ? p : EMPTY_PANES;
    return { layout: fallback, active: fallback };
  }
  let active = p.active;
  if (active === null || !hasLeaf(layout, active)) {
    // The pane nearest to the one that went away.
    const before = leaves(p.layout);
    const from = active === null ? -1 : before.indexOf(active);
    active = before.slice(from + 1).find(exists) ?? before.slice(0, Math.max(from, 0)).reverse().find(exists) ?? leaves(layout)[0];
  }
  return layout === p.layout && active === p.active ? p : { layout, active };
}

/** Moves one split's divider. */
export function setRatio(l: Layout, path: string, ratio: number): Layout {
  if (typeof l === "string") return l;
  if (path === "") {
    const r = clampRatio(ratio);
    return r === l.ratio ? l : { ...l, ratio: r };
  }
  const key = path[0] === "a" ? "a" : "b";
  const child = setRatio(l[key], path.slice(1), ratio);
  return child === l[key] ? l : { ...l, [key]: child };
}

/**
 * Where every pane and divider goes inside `area`. Positions are whole numbers when the area
 * is (pixels): a terminal drawn at half a pixel is blurry.
 */
export function geometry(l: Layout, area: Rect): { panes: PaneRect[]; dividers: Divider[] } {
  const panes: PaneRect[] = [];
  const dividers: Divider[] = [];
  const whole = Number.isInteger(area.w) && Number.isInteger(area.h);
  const walk = (node: Layout, r: Rect, path: string) => {
    if (typeof node === "string") {
      panes.push({ id: node, ...r });
      return;
    }
    const total = node.dir === "row" ? r.w : r.h;
    const first = whole ? Math.round(total * node.ratio) : total * node.ratio;
    if (node.dir === "row") {
      dividers.push({ path, dir: node.dir, ratio: node.ratio, area: r, at: r.x + first });
      walk(node.a, { ...r, w: first }, path + "a");
      walk(node.b, { ...r, x: r.x + first, w: r.w - first }, path + "b");
    } else {
      dividers.push({ path, dir: node.dir, ratio: node.ratio, area: r, at: r.y + first });
      walk(node.a, { ...r, h: first }, path + "a");
      walk(node.b, { ...r, y: r.y + first, h: r.h - first }, path + "b");
    }
  };
  walk(l, area, "");
  return { panes, dividers };
}

function sideOf(r: Rect, edge: Edge, share: number): Rect {
  const w = r.w * share;
  const h = r.h * share;
  if (edge === "left") return { ...r, w };
  if (edge === "right") return { ...r, x: r.x + r.w - w, w };
  if (edge === "top") return { ...r, h };
  return { ...r, y: r.y + r.h - h, h };
}

/**
 * What dropping session `id` at a point of the pane area (w × h, origin top left) would do,
 * and the rectangle to highlight. Null where a drop does nothing.
 */
export function dropAt(
  l: Layout,
  size: { w: number; h: number },
  x: number,
  y: number,
  id: string,
): (DropTarget & { rect: Rect }) | null {
  const { w, h } = size;
  if (w <= 0 || h <= 0 || x < 0 || y < 0 || x >= w || y >= h) return null;
  const ids = leaves(l);
  const moving = ids.includes(id);
  const canSplit = moving ? ids.length > 1 : ids.length < MAX_PANES;
  const whole: Rect = { x: 0, y: 0, w, h };

  if (canSplit && ids.length > 1) {
    const band = Math.min(OUTER_BAND, w / 8, h / 8);
    const edge: Edge | null = x < band ? "left" : x >= w - band ? "right" : y < band ? "top" : y >= h - band ? "bottom" : null;
    if (edge) {
      const rest = moving ? mapLeaves(l, (v) => (v === id ? null : v)) : l;
      if (rest !== null) {
        return { pane: null, zone: edge, rect: sideOf(whole, edge, 1 / (span(rest, dirOf(edge)) + 1)) };
      }
    }
  }

  const pane = geometry(l, whole).panes.find((r) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h);
  if (!pane || pane.id === id) return null;
  const rect: Rect = { x: pane.x, y: pane.y, w: pane.w, h: pane.h };
  if (canSplit) {
    const fx = (x - pane.x) / pane.w;
    const fy = (y - pane.y) / pane.h;
    const distance: [Edge, number][] = [["left", fx], ["right", 1 - fx], ["top", fy], ["bottom", 1 - fy]];
    const [edge, d] = distance.reduce((best, next) => (next[1] < best[1] ? next : best));
    if (d < EDGE_ZONE) return { pane: pane.id, zone: edge, rect: sideOf(rect, edge, 0.5) };
  }
  return { pane: pane.id, zone: "center", rect };
}

/** The pane next to `id` in a direction: the one sharing the most of that edge. */
export function neighbour(l: Layout, id: string, edge: Edge): string | null {
  // Any area will do; whole numbers keep the edges exactly comparable.
  const { panes } = geometry(l, { x: 0, y: 0, w: 100_000, h: 100_000 });
  const from = panes.find((r) => r.id === id);
  if (!from) return null;
  const near = (a: number, b: number) => a === b;
  const overlap = (a0: number, a1: number, b0: number, b1: number) => Math.min(a1, b1) - Math.max(a0, b0);
  let best: { id: string; shared: number } | null = null;
  for (const r of panes) {
    if (r.id === id) continue;
    const touches =
      edge === "left" ? near(r.x + r.w, from.x)
      : edge === "right" ? near(r.x, from.x + from.w)
      : edge === "top" ? near(r.y + r.h, from.y)
      : near(r.y, from.y + from.h);
    if (!touches) continue;
    const shared = dirOf(edge) === "row"
      ? overlap(r.y, r.y + r.h, from.y, from.y + from.h)
      : overlap(r.x, r.x + r.w, from.x, from.x + from.w);
    if (shared > 0 && (!best || shared > best.shared)) best = { id: r.id, shared };
  }
  return best?.id ?? null;
}

/** Reads a saved layout back; anything malformed becomes "no panes". */
export function parsePanes(raw: unknown): Panes {
  if (!raw || typeof raw !== "object") return EMPTY_PANES;
  const { layout, active } = raw as Record<string, unknown>;
  const seen = new Set<string>();
  const read = (v: unknown, depth: number): Layout | null => {
    if (typeof v === "string") {
      if (!v || v.length > 200 || seen.has(v)) return null;
      seen.add(v);
      return v;
    }
    if (!v || typeof v !== "object" || depth > MAX_PANES) return null;
    const s = v as Record<string, unknown>;
    if ((s.dir !== "row" && s.dir !== "col") || typeof s.ratio !== "number") return null;
    const a = read(s.a, depth + 1);
    const b = read(s.b, depth + 1);
    if (a === null || b === null) return null;
    return { dir: s.dir, ratio: clampRatio(s.ratio), a, b };
  };
  const parsed = read(layout, 0);
  if (parsed === null || seen.size > MAX_PANES) return EMPTY_PANES;
  return { layout: parsed, active: typeof active === "string" && seen.has(active) ? active : leaves(parsed)[0] };
}
