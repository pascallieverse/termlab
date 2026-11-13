import { useEffect, useRef, useState, type CSSProperties, type ElementType } from "react";
import { createPortal } from "react-dom";

const SHOW_DELAY_MS = 350;

/**
 * Single-line text that ends in "…" when it doesn't fit, and shows the complete text in a
 * popup on hover — only when it's actually cut off. Screen readers get the full text anyway
 * (truncation is visual only).
 */
export function TruncatedText({
  text,
  className,
  as: Tag = "span",
  style,
}: {
  text: string;
  className?: string;
  as?: ElementType;
  style?: CSSProperties;
}) {
  const ref = useRef<HTMLElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [tip, setTip] = useState<{ x: number; y: number; below: boolean } | null>(null);

  useEffect(() => () => clearTimeout(timer.current), []);

  function show() {
    const el = ref.current;
    if (!el || el.scrollWidth <= el.clientWidth + 1) return;
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      const r = el.getBoundingClientRect();
      const below = r.top < 80;
      setTip({ x: r.left, y: below ? r.bottom + 6 : r.top - 6, below });
    }, SHOW_DELAY_MS);
  }

  function hide() {
    clearTimeout(timer.current);
    setTip(null);
  }

  return (
    <>
      <Tag ref={ref} className={className} style={style} onMouseEnter={show} onMouseLeave={hide}>
        {text}
      </Tag>
      {tip &&
        createPortal(
          <div
            role="tooltip"
            className={`text-tooltip${tip.below ? " below" : ""}`}
            style={{ left: Math.min(tip.x, window.innerWidth - 16), top: tip.y }}
          >
            {text}
          </div>,
          document.body,
        )}
    </>
  );
}
