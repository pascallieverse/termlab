import { useLayoutEffect, useMemo, useRef, type KeyboardEvent, type ReactNode } from "react";

/** A highlighted mirror keeps native textarea selection, undo, and IME behavior. */
export function EnvEditor({ value, onChange, disabled, onKeyDown }: {
  value: string;
  onChange(value: string): void;
  disabled: boolean;
  onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): void;
}) {
  const input = useRef<HTMLTextAreaElement>(null);
  const mirror = useRef<HTMLPreElement>(null);
  const highlighted = useMemo(() => highlightEnv(value.replace(/\r\n?/g, "\n")), [value]);

  function sync() {
    const textarea = input.current;
    const pre = mirror.current;
    if (!textarea || !pre) return;
    // Match the text viewport even with non-overlay scrollbars or manual resizing.
    pre.style.width = `${textarea.clientWidth}px`;
    pre.style.height = `${textarea.clientHeight}px`;
    pre.scrollTop = textarea.scrollTop;
    pre.scrollLeft = textarea.scrollLeft;
  }

  useLayoutEffect(() => {
    const observer = new ResizeObserver(sync);
    if (input.current) observer.observe(input.current);
    sync();
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(sync, [value]);

  return (
    <div className="env-editor">
      <pre ref={mirror} className="env-editor-text env-editor-highlight" aria-hidden="true">
        {highlighted}{" "}
      </pre>
      <textarea
        ref={input}
        id="env-content"
        className="env-editor-text env-editor-input"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onScroll={sync}
        disabled={disabled}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        wrap="off"
        onKeyDown={onKeyDown}
      />
    </div>
  );
}

// Quoted values can span lines; # inside a quoted value is not a comment.
// Rendering text nodes (rather than HTML) keeps file contents literal.
function highlightEnv(text: string): ReactNode[] {
  const tokens = /(^[ \t]*)(export[ \t]+)?([A-Za-z_][\w.-]*)([ \t]*=)|"(?:\\[\s\S]|[^"\\])*(?:"|$)|'(?:[^'])*(?:'|$)|#[^\n]*|\$\{[^}\n]*\}|\$[A-Za-z_][\w]*|\b(?:true|false|null)\b|\b\d+(?:\.\d+)?\b/gm;
  const result: ReactNode[] = [];
  let end = 0;
  for (const match of text.matchAll(tokens)) {
    const start = match.index;
    result.push(text.slice(end, start));
    if (match[3] !== undefined) {
      result.push(
        <span key={start}>
          {match[1]}
          {match[2] && <span className="env-token-keyword">{match[2]}</span>}
          <span className="env-token-key">{match[3]}</span>
          <span className="env-token-equals">{match[4]}</span>
        </span>,
      );
    } else {
      const token = match[0];
      const kind = token.startsWith("#") ? "comment"
        : token.startsWith('"') || token.startsWith("'") ? "string"
        : token.startsWith("$") ? "key"
        : "keyword";
      result.push(<span key={start} className={`env-token-${kind}`}>{token}</span>);
    }
    end = start + match[0].length;
  }
  result.push(text.slice(end));
  return result;
}
