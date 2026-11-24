// Dragging a session (from the sidebar, or a pane by its header) onto the panes.
//
// Done with pointer events rather than HTML drag and drop: Tauri takes drags over to hand
// out file paths (see fileDrop.ts), which turns HTML drag and drop off on Windows.

import { useSyncExternalStore } from "react";
import type { AgentId } from "./api";

export interface SessionDrag {
  id: string;
  title: string;
  agent: AgentId;
  /** Where the pointer is, in window coordinates. */
  x: number;
  y: number;
}

type Dragged = Pick<SessionDrag, "id" | "title" | "agent">;

/** Movement before a press counts as a drag rather than a click. */
const THRESHOLD = 6;

let current: SessionDrag | null = null;
const listeners = new Set<() => void>();
let dropHandler: ((drag: SessionDrag) => void) | null = null;

function set(next: SessionDrag | null) {
  current = next;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** The drag in progress, if any. */
export function useSessionDrag(): SessionDrag | null {
  return useSyncExternalStore(subscribe, () => current);
}

/** Registers what happens when a session is dropped; returns the function that removes it. */
export function onSessionDrop(handler: (drag: SessionDrag) => void): () => void {
  dropHandler = handler;
  return () => {
    if (dropHandler === handler) dropHandler = null;
  };
}

/** The click that follows a drag's release would open the session: it's part of the drag. */
function swallowClick() {
  const stop = (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
  };
  window.addEventListener("click", stop, true);
  setTimeout(() => window.removeEventListener("click", stop, true), 0);
}

/** Call from `onPointerDown`. The drag starts once the pointer has moved a little. */
export function beginSessionDrag(e: { nativeEvent: PointerEvent; currentTarget: Element }, session: Dragged) {
  const down = e.nativeEvent;
  if (down.button !== 0 || !down.isPrimary || down.pointerType === "touch" || current) return;
  const source = e.currentTarget;
  const start = { x: down.clientX, y: down.clientY };
  let last = start;
  let frame = 0;

  const move = (ev: PointerEvent) => {
    last = { x: ev.clientX, y: ev.clientY };
    if (!current) {
      if (Math.hypot(last.x - start.x, last.y - start.y) < THRESHOLD) return;
      // Keep receiving the pointer when it leaves the window.
      try {
        source.setPointerCapture(down.pointerId);
      } catch {
        // The element is gone: the window listeners still follow the pointer inside the app.
      }
      document.body.classList.add("session-dragging");
      set({ id: session.id, title: session.title, agent: session.agent, ...last });
      return;
    }
    if (!frame) {
      frame = requestAnimationFrame(() => {
        frame = 0;
        if (current) set({ ...current, ...last });
      });
    }
  };

  /** `released`: the button is up. Esc and a lost window end the drag with it still down. */
  const finish = (drop: boolean, released: boolean) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("blur", cancel);
    window.removeEventListener("keydown", key, true);
    cancelAnimationFrame(frame);
    if (!current) return;
    const drag = { ...current, ...last };
    document.body.classList.remove("session-dragging");
    set(null);
    try {
      source.releasePointerCapture(down.pointerId);
    } catch {
      // Already released.
    }
    if (released) swallowClick();
    else window.addEventListener("pointerup", swallowClick, { once: true, capture: true });
    if (drop) dropHandler?.(drag);
  };
  const up = (ev: PointerEvent) => {
    last = { x: ev.clientX, y: ev.clientY };
    finish(true, true);
  };
  const cancel = () => finish(false, false);
  const key = (ev: KeyboardEvent) => {
    if (ev.key !== "Escape" || !current) return;
    // Esc cancels the drag, and must not also reach the agent in the terminal.
    ev.preventDefault();
    ev.stopPropagation();
    cancel();
  };

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("blur", cancel);
  window.addEventListener("keydown", key, true);
}
