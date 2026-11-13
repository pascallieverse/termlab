import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { relativeTime, tildify } from "./agents";
import { useStore } from "./store";
import { Icon } from "./ui";

/** Above this many folders, a filter box appears. */
const FILTER_FROM = 6;

/**
 * Picks the project folder for a new session: a compact list, most recently used first,
 * with what's going on in each folder. Scrolls after a few rows so the rest of the form
 * stays in view.
 */
export function FolderPicker({
  value,
  onChange,
  onChooseOther,
}: {
  value: number | null;
  onChange(id: number): void;
  onChooseOther(): void;
}) {
  const store = useStore();
  const [query, setQuery] = useState("");
  const list = useRef<HTMLDivElement>(null);

  // What's happening in each folder.
  const stats = useMemo(() => {
    const m = new Map<number, { open: number; last: number }>();
    for (const s of store.sessions) {
      const cur = m.get(s.folderId) ?? { open: 0, last: 0 };
      if (s.live) cur.open += 1;
      cur.last = Math.max(cur.last, s.lastActivityAt);
      m.set(s.folderId, cur);
    }
    return m;
  }, [store.sessions]);

  // Most recently used first. The order is fixed when the picker opens, so rows don't jump
  // around while sessions keep working; a folder added meanwhile goes on top.
  const [rank] = useState(() => {
    const sorted = [...store.folders].sort(
      (a, b) => (stats.get(b.id)?.last ?? 0) - (stats.get(a.id)?.last ?? 0) || a.name.localeCompare(b.name),
    );
    return new Map(sorted.map((f, i) => [f.id, i]));
  });
  const ordered = useMemo(
    () => [...store.folders].sort((a, b) => (rank.get(a.id) ?? -1) - (rank.get(b.id) ?? -1)),
    [store.folders, rank],
  );

  const q = query.trim().toLowerCase();
  const shown = q
    ? ordered.filter((f) => f.name.toLowerCase().includes(q) || f.path.toLowerCase().includes(q))
    : ordered;

  // Keep the chosen folder in view (when the picker opens, and after choosing another).
  useEffect(() => {
    list.current?.querySelector('[aria-checked="true"]')?.scrollIntoView({ block: "nearest" });
  }, [value]);

  function focusRow(index: number) {
    const rows = list.current?.querySelectorAll<HTMLButtonElement>('[role="radio"]');
    rows?.[index]?.focus();
  }

  // Arrow keys move through the list and choose, as in any group of options.
  function onRowKey(e: KeyboardEvent, index: number) {
    const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (step === 0) return;
    e.preventDefault();
    const next = Math.min(shown.length - 1, Math.max(0, index + step));
    onChange(shown[next].id);
    focusRow(next);
  }

  function onFilterKey(e: KeyboardEvent) {
    if (e.key === "Enter" && shown.length > 0) {
      e.preventDefault();
      onChange(shown[0].id);
    } else if (e.key === "ArrowDown" && shown.length > 0) {
      e.preventDefault();
      onChange(shown[0].id);
      focusRow(0);
    }
  }

  return (
    <div className="field">
      <div className="folder-picker-head">
        <span className="label" id="ns-folder">
          Project folder
        </span>
        <button type="button" className="link-btn small folder-other" onClick={onChooseOther}>
          <Icon name="folderPlus" size={13} /> Choose another folder…
        </button>
      </div>
      <div className="folder-picker">
        {store.folders.length >= FILTER_FROM && (
          <div className="folder-filter">
            <Icon name="search" />
            <label htmlFor="ns-folder-q" className="sr">
              Filter folders
            </label>
            <input
              id="ns-folder-q"
              placeholder="Filter folders"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onFilterKey}
            />
          </div>
        )}
        <div role="radiogroup" aria-labelledby="ns-folder" className="folder-list" ref={list}>
          {shown.map((f, i) => {
            const st = stats.get(f.id);
            const on = value === f.id;
            return (
              <button
                type="button"
                key={f.id}
                role="radio"
                aria-checked={on}
                tabIndex={on || (value === null && i === 0) ? 0 : -1}
                className={`folder-row${on ? " on" : ""}`}
                onClick={() => onChange(f.id)}
                onKeyDown={(e) => onRowKey(e, i)}
              >
                <span className="folder-row-mark" aria-hidden="true" />
                <span className="folder-row-name">{f.name}</span>
                <span className="mono folder-row-path">{tildify(f.path, store.home)}</span>
                <span className="folder-row-meta">
                  {st && st.open > 0 ? (
                    <span className="folder-row-open">{st.open} open</span>
                  ) : st && st.last > 0 ? (
                    relativeTime(st.last)
                  ) : (
                    "no sessions"
                  )}
                </span>
              </button>
            );
          })}
          {shown.length === 0 && (
            <p className="folder-none muted small">
              {store.folders.length === 0 ? "No project folders yet." : "No folder matches."}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
