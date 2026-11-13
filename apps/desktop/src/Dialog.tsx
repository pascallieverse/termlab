import { useEffect, useId, useRef, type ReactNode } from "react";

/** A modal dialog: focus moves in and is trapped; Esc and the backdrop close it. */
export function Dialog({
  title,
  onClose,
  children,
  width = 720,
  className = "",
  labelledBy,
}: {
  title?: string;
  onClose(): void;
  children: ReactNode;
  width?: number;
  className?: string;
  labelledBy?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const restore = useRef<Element | null>(null);

  useEffect(() => {
    restore.current = document.activeElement;
    const el = ref.current;
    const first = el?.querySelector<HTMLElement>("[data-autofocus]") ?? focusables(el)[0];
    first?.focus();
    return () => {
      if (restore.current instanceof HTMLElement) restore.current.focus();
    };
  }, []);

  return (
    <div
      className="overlay"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy ?? (title ? titleId : undefined)}
        className={`dialog ${className}`}
        style={{ width: `min(${width}px, calc(100vw - 32px))` }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            onClose();
          } else if (e.key === "Tab") {
            const list = focusables(ref.current);
            if (list.length === 0) return;
            const i = list.indexOf(document.activeElement as HTMLElement);
            if (e.shiftKey && i <= 0) {
              e.preventDefault();
              list[list.length - 1].focus();
            } else if (!e.shiftKey && i === list.length - 1) {
              e.preventDefault();
              list[0].focus();
            }
          }
        }}
      >
        {title && (
          <h1 id={titleId} className="sr">
            {title}
          </h1>
        )}
        {children}
      </div>
    </div>
  );
}

function focusables(root: HTMLElement | null): HTMLElement[] {
  if (!root) return [];
  return Array.from(
    root.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((el) => el.offsetParent !== null);
}
