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

  if (error) return <div className="pane-message">{error}</div>;
  if (text === null) return <div className="pane-message">Loading diff…</div>;

  return (
    <div className="diff">
      <div className="diff-toolbar">
        <span>
          {files.length} file{files.length === 1 ? "" : "s"} changed since the session started
        </span>
        <div className="spacer" />
        <button className="btn small" onClick={() => setVersion((v) => v + 1)}>
          Refresh
        </button>
      </div>
      {files.length === 0 && <div className="pane-message">No changes yet.</div>}
      {files.map((f) => (
        <section key={f.path} className="diff-file" aria-label={f.path}>
          <header className="diff-file-head mono">
            <span className="diff-path">{f.path}</span>
            <span className="add">+{f.added}</span>
            <span className="del">−{f.deleted}</span>
          </header>
          {f.binary ? (
            <div className="pane-message">Binary file</div>
          ) : (
            <table className="diff-table mono">
              <tbody>
                {f.rows.map((row, i) =>
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
          )}
        </section>
      ))}
    </div>
  );
}
