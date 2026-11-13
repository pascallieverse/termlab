import { useEffect, useRef, useState } from "react";
import type { Folder } from "./api";

type Target = { id: number; after: boolean };
type Drag = { id: number; x: number; y: number; startY: number; active: boolean; target: Target | null };

export function useProjectOrder(folders: Folder[], move: (id: number, before: number | null) => Promise<void>) {
  const latest = useRef({ folders, move });
  latest.current = { folders, move };
  const list = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const busy = useRef(false);
  const [indicator, setIndicator] = useState<Target | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [announcement, setAnnouncement] = useState("");

  async function commit(id: number, target: Target) {
    if (busy.current || id === target.id) return;
    const { folders, move } = latest.current;
    const remaining = folders.filter((f) => f.id !== id);
    const index = remaining.findIndex((f) => f.id === target.id);
    if (index < 0) return;
    const before = remaining[index + (target.after ? 1 : 0)]?.id ?? null;
    const oldIndex = folders.findIndex((f) => f.id === id);
    if (oldIndex < 0 || (folders[oldIndex + 1]?.id ?? null) === before) return;
    busy.current = true;
    try {
      await move(id, before);
    } finally {
      busy.current = false;
    }
  }

  useEffect(() => {
    let frame = 0;
    function reset() {
      drag.current = null;
      setDragging(null);
      setIndicator(null);
      cancelAnimationFrame(frame);
    }
    function updateTarget(scroll: boolean) {
      const d = drag.current;
      const container = list.current;
      if (!d || !container) return;
      const rect = container.getBoundingClientRect();
      if (d.active && d.x >= rect.left && d.x <= rect.right && d.y >= rect.top && d.y <= rect.bottom) {
        const speed = d.y < rect.top + 32 ? -8 : d.y > rect.bottom - 32 ? 8 : 0;
        if (scroll && speed) container.scrollTop += speed;
        const headers = Array.from(container.querySelectorAll<HTMLElement>("[data-folder-head]"));
        // Use project headers as anchors, keeping every project's sessions together.
        const head = headers.find((el) => d.y < el.getBoundingClientRect().bottom) ?? headers.at(-1);
        if (head) {
          const r = head.getBoundingClientRect();
          d.target = { id: Number(head.dataset.folderHead), after: d.y > r.top + r.height / 2 };
          const target = d.target.id === d.id ? null : d.target;
          setIndicator((prev) => prev?.id === target?.id && prev?.after === target?.after ? prev : target);
        }
      } else {
        d.target = null;
        setIndicator(null);
      }
    }
    function tick() {
      updateTarget(true);
      if (drag.current) frame = requestAnimationFrame(tick);
    }
    function pointerMove(e: PointerEvent) {
      const d = drag.current;
      if (!d) return;
      d.x = e.clientX;
      d.y = e.clientY;
      if (!d.active && Math.abs(d.y - d.startY) > 4) {
        d.active = true;
        setDragging(d.id);
      }
    }
    function pointerUp(e: PointerEvent) {
      pointerMove(e);
      updateTarget(false);
      const d = drag.current;
      reset();
      if (d?.active && d.target) void commit(d.id, d.target);
    }
    function keyDown(e: KeyboardEvent) {
      if (e.key === "Escape" && drag.current) {
        e.preventDefault();
        reset();
      }
    }
    function start() {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(tick);
    }
    window.addEventListener("pointermove", pointerMove);
    window.addEventListener("pointerup", pointerUp);
    window.addEventListener("pointercancel", reset);
    window.addEventListener("blur", reset);
    window.addEventListener("keydown", keyDown);
    // The handle starts the loop after populating drag.current.
    const container = list.current;
    container?.addEventListener("pointerdown", start);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("pointermove", pointerMove);
      window.removeEventListener("pointerup", pointerUp);
      window.removeEventListener("pointercancel", reset);
      window.removeEventListener("blur", reset);
      window.removeEventListener("keydown", keyDown);
      container?.removeEventListener("pointerdown", start);
    };
  }, []);

  // Announce the committed order, including changes from another window.
  useEffect(() => {
    setAnnouncement(folders.length ? `Project order: ${folders.map((f) => f.name).join(", ")}` : "");
  }, [folders]);

  return {
    list, indicator, dragging, announcement,
    handle: (folder: Folder) => (
      <button
        className="folder-drag"
        aria-label={`Reorder ${folder.name}`}
        title="Drag to reorder · Alt+↑/↓ to move"
        disabled={folders.length < 2}
        onPointerDown={(e) => {
          if (e.button !== 0 || !e.isPrimary || busy.current) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = { id: folder.id, x: e.clientX, y: e.clientY, startY: e.clientY, active: false, target: null };
        }}
        onKeyDown={(e) => {
          if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
          e.preventDefault();
          const index = folders.findIndex((f) => f.id === folder.id);
          const after = e.key === "ArrowDown";
          const target = folders[index + (after ? 1 : -1)];
          if (target) void commit(folder.id, { id: target.id, after });
        }}
      >
        <svg width="10" height="16" viewBox="0 0 10 16" fill="currentColor" aria-hidden="true">
          {[4, 8, 12].flatMap((y) => [3, 7].map((x) => <circle key={`${x}-${y}`} cx={x} cy={y} r="1" />))}
        </svg>
      </button>
    ),
  };
}
