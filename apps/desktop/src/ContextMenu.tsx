import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";

export type MenuItem =
  | {
      label: string;
      onSelect(): void;
      danger?: boolean;
      disabled?: boolean;
      /** Shown as the item's tooltip, e.g. why it's disabled. */
      hint?: string;
      shortcut?: string;
    }
  | "separator";

interface OpenMenu {
  x: number;
  y: number;
  items: MenuItem[];
  label: string;
}

/**
 * A right-click menu. `open` takes the triggering event (mouse or the keyboard's context-menu
 * key) and the items; the returned element renders the menu when open.
 */
export function useContextMenu() {
  const [menu, setMenu] = useState<OpenMenu | null>(null);

  function open(e: ReactMouseEvent, label: string, items: MenuItem[]) {
    e.preventDefault();
    e.stopPropagation();
    let { clientX: x, clientY: y } = e;
    // Keyboard-triggered (Shift+F10 / context-menu key): anchor to the element.
    if (x === 0 && y === 0) {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      x = r.left + 24;
      y = r.bottom - 4;
    }
    setMenu({ x, y, items, label });
  }

  const element = menu ? <Menu menu={menu} onClose={() => setMenu(null)} /> : null;
  return { open, element };
}

function Menu({ menu, onClose }: { menu: OpenMenu; onClose(): void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: menu.x, y: menu.y });
  const restore = useRef<Element | null>(document.activeElement);

  // Keep the menu on screen.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.max(8, Math.min(menu.x, window.innerWidth - r.width - 8)),
      y: Math.max(8, Math.min(menu.y, window.innerHeight - r.height - 8)),
    });
    el.querySelector<HTMLElement>('[role="menuitem"]:not([disabled])')?.focus();
  }, [menu]);

  useEffect(() => {
    const close = () => onClose();
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("wheel", close, { passive: true });
    const r = restore.current;
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("wheel", close);
      if (r instanceof HTMLElement) r.focus();
    };
  }, [onClose]);

  function move(delta: number) {
    const items = Array.from(ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not([disabled])') ?? []);
    if (items.length === 0) return;
    const i = items.indexOf(document.activeElement as HTMLElement);
    items[(i + delta + items.length) % items.length].focus();
  }

  return (
    <div
      ref={ref}
      role="menu"
      aria-label={menu.label}
      className="context-menu"
      style={{ left: pos.x, top: pos.y }}
      onContextMenu={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        if (e.key === "Escape" || e.key === "Tab") {
          e.preventDefault();
          onClose();
        } else if (e.key === "ArrowDown") {
          e.preventDefault();
          move(1);
        } else if (e.key === "ArrowUp") {
          e.preventDefault();
          move(-1);
        }
      }}
    >
      {menu.items.map((item, i) =>
        item === "separator" ? (
          <div key={i} role="separator" className="context-sep" />
        ) : (
          <button
            key={i}
            role="menuitem"
            className={`context-item${item.danger ? " danger" : ""}`}
            disabled={item.disabled}
            title={item.hint}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            <span>{item.label}</span>
            {item.shortcut && <span className="kbd-hint mono">{item.shortcut}</span>}
          </button>
        ),
      )}
    </div>
  );
}
