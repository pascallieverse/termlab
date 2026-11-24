import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { dropAt, geometry, type Divider as DividerLine, type DropTarget, type Rect } from "./paneLayout";
import { SessionPane } from "./SessionPane";
import { onSessionDrop, useSessionDrag, type SessionDrag } from "./sessionDrag";
import { useStore } from "./store";
import { Monogram } from "./ui";

/** A divider can't make a pane smaller than this (in pixels), while the window allows it. */
const MIN_PANE = { w: 240, h: 130 };
const KEY_STEP = 0.03;

/** Before the area has been measured, panes are laid out on this grid and placed in percent. */
const UNMEASURED = 10_000;
const px = (r: Rect): CSSProperties => ({ left: r.x, top: r.y, width: r.w, height: r.h });
const percent = (r: Rect): CSSProperties => ({
  left: `${r.x / (UNMEASURED / 100)}%`, top: `${r.y / (UNMEASURED / 100)}%`,
  width: `${r.w / (UNMEASURED / 100)}%`, height: `${r.h / (UNMEASURED / 100)}%`,
});
const FULL: CSSProperties = { left: 0, top: 0, width: "100%", height: "100%" };

/**
 * The focus view: one session, or several next to and above each other. Sessions are
 * dragged in from the sidebar (an edge splits, the middle replaces), panes are moved by
 * their header, and the lines between them resize.
 */
export function Panes() {
  const store = useStore();
  const host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const drag = useSessionDrag();
  const { layout, paneIds, selectedId } = store;
  const multi = paneIds.length > 1;

  useLayoutEffect(() => {
    const el = host.current;
    if (!el) return;
    const measure = () =>
      setSize((prev) => (prev.w === el.clientWidth && prev.h === el.clientHeight ? prev : { w: el.clientWidth, h: el.clientHeight }));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const measured = size.w > 0 && size.h > 0;
  const geo = useMemo(
    () => (layout === null ? null : geometry(layout, { x: 0, y: 0, ...(measured ? size : { w: UNMEASURED, h: UNMEASURED }) })),
    [layout, size, measured],
  );
  const place = measured ? px : percent;

  // The active pane has the keyboard. A pane that just opened focuses its own terminal;
  // this covers panes already on screen (picked in the sidebar, the palette, by shortcut).
  useEffect(() => {
    if (!selectedId || document.querySelector('[role="dialog"]')) return;
    const pane = Array.from(host.current?.children ?? []).find((el) => (el as HTMLElement).dataset.pane === selectedId);
    if (!pane || pane.contains(document.activeElement)) return;
    pane.querySelector<HTMLElement>(".xterm-helper-textarea")?.focus();
  }, [selectedId]);

  const target = (d: SessionDrag) => {
    const box = host.current?.getBoundingClientRect();
    if (!box || layout === null) return null;
    return dropAt(layout, { w: box.width, h: box.height }, d.x - box.left, d.y - box.top, d.id);
  };
  const drop = useRef<(d: SessionDrag) => void>(() => {});
  drop.current = (d) => {
    const t = target(d);
    if (t) store.placePane(d.id, { pane: t.pane, zone: t.zone });
  };
  useEffect(() => onSessionDrop((d) => drop.current(d)), []);

  const hover = drag ? target(drag) : null;
  const sessions = new Map(store.visibleSessions.map((s) => [s.id, s]));
  // A fixed order in the page, whatever the arrangement: a pane that moves keeps its
  // terminal (and the keyboard) instead of being taken out and put back.
  const ordered = [...(geo?.panes ?? [])].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

  return (
    <div ref={host} className="panes">
      {ordered.map((r) => {
        const s = sessions.get(r.id);
        if (!s) return null;
        const active = r.id === selectedId;
        const style: CSSProperties = multi ? place(r) : FULL;
        return (
          <div key={r.id} data-pane={r.id} className="pane-slot" style={style}>
            <SessionPane session={s} active={active} multi={multi} />
          </div>
        );
      })}
      {/* Outer splits last, so that where two lines meet the longer one is the one you grab. */}
      {multi && measured && [...(geo?.dividers ?? [])].reverse().map((d) => <Divider key={d.path} line={d} />)}
      {drag && (
        <div className={`drop-layer${hover ? "" : " is-waiting"}`}>
          {hover ? (
            <div className="drop-zone" style={px(hover.rect)}>
              <span>{dropLabel(hover, paneIds.includes(drag.id))}</span>
            </div>
          ) : (
            // From the moment a drag starts, show where it can go.
            <div className="drop-hint">
              <span>
                {paneIds.length === 1 && paneIds[0] === drag.id
                  ? "Already open here. Drag another session to split"
                  : "Drop here: an edge splits, the middle replaces"}
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function dropLabel(t: DropTarget, moving: boolean): string {
  if (t.zone === "center") return moving ? "Swap panes" : "Replace";
  const side = { left: "left", right: "right", top: "above", bottom: "below" }[t.zone];
  return `${moving ? "Move" : "Open"} ${t.zone === "left" || t.zone === "right" ? `on the ${side}` : side}`;
}

/** The line between two panes: drag to resize, double-click for equal halves, arrow keys. */
function Divider({ line }: { line: DividerLine }) {
  const store = useStore();
  const row = line.dir === "row";
  const total = row ? line.area.w : line.area.h;
  const start = row ? line.area.x : line.area.y;
  const dragging = useRef(false);
  const [active, setActive] = useState(false);

  const set = (ratio: number) => {
    const min = Math.min(0.5, (row ? MIN_PANE.w : MIN_PANE.h) / Math.max(total, 1));
    store.resizePanes(line.path, Math.min(1 - min, Math.max(min, ratio)));
  };
  const stop = () => {
    dragging.current = false;
    setActive(false);
  };

  return (
    <div
      className={`pane-divider ${row ? "row" : "col"}${active ? " is-active" : ""}`}
      style={row ? { left: line.at, top: line.area.y, height: line.area.h } : { top: line.at, left: line.area.x, width: line.area.w }}
      role="separator"
      tabIndex={0}
      aria-orientation={row ? "vertical" : "horizontal"}
      aria-label="Resize panes"
      aria-valuemin={10}
      aria-valuemax={90}
      aria-valuenow={Math.round(line.ratio * 100)}
      title="Drag to resize · Double-click for equal halves"
      onPointerDown={(e) => {
        if (e.button !== 0 || !e.isPrimary) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        dragging.current = true;
        setActive(true);
      }}
      onPointerMove={(e) => {
        if (!dragging.current) return;
        const box = e.currentTarget.parentElement?.getBoundingClientRect();
        if (!box) return;
        set(((row ? e.clientX - box.left : e.clientY - box.top) - start) / Math.max(total, 1));
      }}
      onPointerUp={stop}
      onPointerCancel={stop}
      onLostPointerCapture={stop}
      onDoubleClick={() => store.resizePanes(line.path, 0.5)}
      onKeyDown={(e) => {
        const back = row ? "ArrowLeft" : "ArrowUp";
        const forward = row ? "ArrowRight" : "ArrowDown";
        if (e.key !== back && e.key !== forward && e.key !== "Enter") return;
        e.preventDefault();
        if (e.key === "Enter") store.resizePanes(line.path, 0.5);
        else set(line.ratio + (e.key === forward ? KEY_STEP : -KEY_STEP));
      }}
    />
  );
}

/** What's being dragged, next to the pointer. */
export function SessionDragGhost() {
  const drag = useSessionDrag();
  if (!drag) return null;
  return (
    <div className="drag-ghost" style={{ transform: `translate(${drag.x + 14}px, ${drag.y + 12}px)` }} aria-hidden="true">
      <Monogram agent={drag.agent} size={20} />
      <span>{drag.title}</span>
    </div>
  );
}
