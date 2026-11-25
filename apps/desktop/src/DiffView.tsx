import { useEffect, useMemo, useState } from "react";
import { api, errorText, type SessionStatus } from "./api";

interface Row {
  kind: "context" | "change" | "hunk";
  left?: { n: number; text: string; del: boolean };
  right?: { n: number; text: string; add: boolean };
  header?: string;
}

interface FileDiff {
  path: string;
  rows: Row[];
  added: number;
  deleted: number;
  binary: boolean;
}

/** Parses `git diff` output into side-by-side rows per file. */
export function parseDiff(text: string): FileDiff[] {
  const files: FileDiff[] = [];
  let file: FileDiff | null = null;
  let l = 0;
  let r = 0;
  let dels: string[] = [];
  let adds: string[] = [];
  const flush = () => {
    if (!file) return;
    const n = Math.max(dels.length, adds.length);
    for (let i = 0; i < n; i++) {
      file.rows.push({
        kind: "change",
        left: i < dels.length ? { n: l++, text: dels[i], del: true } : undefined,
        right: i < adds.length ? { n: r++, text: adds[i], add: true } : undefined,
      });
    }
    dels = [];
    adds = [];
  };
  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) {
      flush();
      const m = / b\/(.*)$/.exec(line);
      file = { path: m ? m[1] : line, rows: [], added: 0, deleted: 0, binary: false };
      files.push(file);
      continue;
    }
    if (!file) continue;
    if (line.startsWith("+++ ")) {
      const p = line.slice(4);
      if (p !== "/dev/null") file.path = p.replace(/^b\//, "");
      continue;
    }
    if (line.startsWith("--- ") || line.startsWith("index ") || line.startsWith("new file") || line.startsWith("deleted file") || line.startsWith("similarity") || line.startsWith("old mode") || line.startsWith("new mode")) {
      continue;
    }
    if (line.startsWith("Binary files")) {
      file.binary = true;
      continue;
    }
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(line);
    if (hunk) {
      flush();
      l = Number(hunk[1]);
      r = Number(hunk[2]);
      file.rows.push({ kind: "hunk", header: line });
      continue;
    }
    if (line.startsWith("-")) {
      dels.push(line.slice(1));
      file.deleted++;
    } else if (line.startsWith("+")) {
      adds.push(line.slice(1));
      file.added++;
    } else if (line.startsWith(" ")) {
      flush();
      file.rows.push({
        kind: "context",
        left: { n: l++, text: line.slice(1), del: false },
        right: { n: r++, text: line.slice(1), add: false },
      });
    }
  }
  flush();
  return files;
}

/** Rows rendered up front across all files; files past it start collapsed. A diff of
 * thousands of files or lines as one big table takes the WebView down. */
const ROW_BUDGET = 3000;
/** Rows shown per file at a time; "Show more" adds this many. */
const FILE_PAGE = 1000;

/** How many rows each file shows at first: files fill the budget in order, the rest wait for a click. */
function initialShown(files: FileDiff[]): Map<string, number> {
  const shown = new Map<string, number>();
  let budget = ROW_BUDGET;
  for (const f of files) {
    const n = f.binary ? 0 : Math.min(f.rows.length, FILE_PAGE);
    if (n <= budget) {
      shown.set(f.path, n);
      budget -= n;
    } else {
      shown.set(f.path, 0);
      budget = 0;
    }
  }
  return shown;
}

/** Read-only side-by-side diff since the session's start commit. */
export function DiffView({ sessionId, status }: { sessionId: string; status: SessionStatus }) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);

  useEffect(() => {
    let cancelled = false;
    api.diff(sessionId).then(
      (d) => {
        if (!cancelled) {
          setText(d);
          setError(null);
        }
      },
      (e) => !cancelled && setError(errorText(e)),
    );
    return () => {
      cancelled = true;
    };
  }, [sessionId, status, version]);

  const files = useMemo(() => (text ? parseDiff(text) : []), [text]);
  const truncated = useMemo(() => !!text && text.trimEnd().endsWith("… diff truncated"), [text]);
  // Rows shown per file; 0 = collapsed. Keeps what the user opened across refreshes.
  const [shown, setShown] = useState<Map<string, number>>(new Map());
  useEffect(() => {
    setShown((prev) => {
      const next = initialShown(files);
      for (const f of files) {
        const p = prev.get(f.path);
        if (p !== undefined) next.set(f.path, Math.min(p, f.rows.length));
      }
      return next;
    });
  }, [files]);

  const setFile = (path: string, n: number) =>
    setShown((prev) => {
      const next = new Map(prev);
      next.set(path, n);
      return next;
    });

  if (error) return <div className="pane-message">{error}</div>;
  if (text === null) return <div className="pane-message">Loading diff…</div>;

  return (
    <div className="diff">
      <div className="diff-toolbar">
        <span>
          {files.length} file{files.length === 1 ? "" : "s"} changed since the session started
          {truncated && " (diff too large, showing the first part)"}
        </span>
        <div className="spacer" />
        <button className="btn small" onClick={() => setVersion((v) => v + 1)}>
          Refresh
        </button>
      </div>
      {files.length === 0 && <div className="pane-message">No changes yet.</div>}
      {files.map((f) => {
        const n = f.binary ? 0 : (shown.get(f.path) ?? 0);
        const open = n > 0;
        return (
          <section key={f.path} className="diff-file" aria-label={f.path}>
            <header
              className={`diff-file-head mono${f.binary ? "" : " toggle"}`}
              role={f.binary ? undefined : "button"}
              tabIndex={f.binary ? undefined : 0}
              aria-expanded={f.binary ? undefined : open}
              onClick={() => !f.binary && setFile(f.path, open ? 0 : Math.min(f.rows.length, FILE_PAGE))}
              onKeyDown={(e) => {
                if (!f.binary && (e.key === "Enter" || e.key === " ")) {
                  e.preventDefault();
                  setFile(f.path, open ? 0 : Math.min(f.rows.length, FILE_PAGE));
                }
              }}
            >
              {!f.binary && <span className="diff-caret">{open ? "▾" : "▸"}</span>}
              <span className="diff-path">{f.path}</span>
              <span className="add">+{f.added}</span>
              <span className="del">−{f.deleted}</span>
            </header>
            {f.binary ? (
              <div className="pane-message">Binary file</div>
            ) : open ? (
              <>
                <table className="diff-table mono">
                  <tbody>
                    {f.rows.slice(0, n).map((row, i) =>
                      row.kind === "hunk" ? (
                        <tr key={i} className="hunk">
                          <td colSpan={4}>{row.header}</td>
                        </tr>
                      ) : (
                        <tr key={i}>
                          <td className="ln">{row.left?.n ?? ""}</td>
                          <td className={row.left?.del ? "del-line" : row.left ? "" : "empty"}>{row.left?.text ?? ""}</td>
                          <td className="ln">{row.right?.n ?? ""}</td>
                          <td className={row.right?.add ? "add-line" : row.right ? "" : "empty"}>{row.right?.text ?? ""}</td>
                        </tr>
                      ),
                    )}
                  </tbody>
                </table>
                {n < f.rows.length && (
                  <div className="diff-more">
                    <span>
                      {f.rows.length - n} more line{f.rows.length - n === 1 ? "" : "s"}
                    </span>
                    <button className="btn small" onClick={() => setFile(f.path, Math.min(f.rows.length, n + FILE_PAGE))}>
                      Show more
                    </button>
                  </div>
                )}
              </>
            ) : null}
          </section>
        );
      })}
    </div>
  );
}
