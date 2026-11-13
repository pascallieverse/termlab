import { useEffect, useMemo, useState } from "react";
import { api, errorText, type SessionStatus, type TimelineEntry } from "./api";
import { Icon } from "./ui";

type Kind = TimelineEntry["kind"];

const KINDS: { kind: Kind; label: string; filterLabel: string; hint: string }[] = [
  { kind: "prompt", label: "You", filterLabel: "You", hint: "Your prompts" },
  { kind: "message", label: "Agent", filterLabel: "Agent", hint: "The agent's messages" },
  { kind: "tool", label: "Tool", filterLabel: "Tools", hint: "Commands, file reads and edits, searches" },
  { kind: "hook", label: "Event", filterLabel: "Events", hint: "Permission requests, notifications, turn ends" },
];

const HIDDEN_KEY = "termlab.timeline.hidden";

/** Kinds you've hidden are a per-viewer preference, kept in localStorage. */
function loadHidden(): Set<Kind> {
  try {
    return new Set(JSON.parse(localStorage.getItem(HIDDEN_KEY) ?? "[]") as Kind[]);
  } catch {
    return new Set();
  }
}

function saveHidden(hidden: Set<Kind>) {
  try {
    localStorage.setItem(HIDDEN_KEY, JSON.stringify([...hidden]));
  } catch {
    // Storage unavailable: the filter still applies in this window.
  }
}

/** Hook events plus tool calls and messages from the agent's transcript, with filters. */
export function TimelineView({
  sessionId,
  status,
  subagentId,
}: {
  sessionId: string;
  status: SessionStatus;
  /** Show this sub-agent's steps instead of the session's. */
  subagentId?: string;
}) {
  const [entries, setEntries] = useState<TimelineEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hidden, setHidden] = useState<Set<Kind>>(loadHidden);
  const [query, setQuery] = useState("");

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      (subagentId ? api.subagentTimeline(sessionId, subagentId) : api.timeline(sessionId)).then(
        (t) => !cancelled && setEntries(t),
        (e) => !cancelled && setError(errorText(e)),
      );
    void load();
    const timer = status === "running" ? setInterval(load, 4000) : undefined;
    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, [sessionId, status, subagentId]);

  const counts = useMemo(() => {
    const c: Record<Kind, number> = { prompt: 0, message: 0, tool: 0, hook: 0 };
    for (const e of entries ?? []) c[e.kind]++;
    return c;
  }, [entries]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (entries ?? []).filter((e) => !hidden.has(e.kind) && (!q || e.text.toLowerCase().includes(q)));
  }, [entries, hidden, query]);

  function toggle(kind: Kind) {
    const next = new Set(hidden);
    if (next.has(kind)) next.delete(kind);
    else next.add(kind);
    setHidden(next);
    saveHidden(next);
  }

  if (error) return <div className="pane-message">{error}</div>;
  if (entries === null) return <div className="pane-message">Loading timeline…</div>;
  if (entries.length === 0)
    return (
      <div className="pane-message">
        Nothing recorded yet. The timeline fills from hook events and the agent's transcript, where the agent
        provides them.
      </div>
    );

  const filtered = hidden.size > 0 || query.trim() !== "";

  return (
    <div className="timeline-pane">
      <div className="timeline-toolbar">
        <div role="group" aria-label="Show in timeline" className="timeline-kinds">
          {KINDS.map(({ kind, filterLabel, hint }) => (
            <button
              key={kind}
              className={`chip tl-chip tl-chip-${kind}${hidden.has(kind) ? "" : " on"}`}
              aria-pressed={!hidden.has(kind)}
              title={`${hidden.has(kind) ? "Show" : "Hide"}: ${hint}`}
              onClick={() => toggle(kind)}
            >
              <span className="tl-chip-dot" aria-hidden="true" />
              {filterLabel}
              <span className="filter-count">{counts[kind]}</span>
            </button>
          ))}
        </div>
        <div className="search-field timeline-search">
          <Icon name="search" />
          <label htmlFor="tl-q" className="sr">
            Search the timeline
          </label>
          <input id="tl-q" placeholder="Search" value={query} onChange={(e) => setQuery(e.target.value)} />
          {query && (
            <button className="icon-btn small" aria-label="Clear search" onClick={() => setQuery("")}>
              <Icon name="close" />
            </button>
          )}
        </div>
        {filtered && (
          <span className="muted small timeline-shown">
            {visible.length} of {entries.length}
          </span>
        )}
      </div>

      {visible.length === 0 ? (
        <div className="pane-message">
          Nothing matches.{" "}
          <button
            className="link-btn"
            onClick={() => {
              setQuery("");
              setHidden(new Set());
              saveHidden(new Set());
            }}
          >
            Show everything
          </button>
        </div>
      ) : (
        <ol className="timeline">
          {visible.map((e, i) => (
            <li key={i} className={`tl-${e.kind}`}>
              <time className="mono">
                {e.at
                  ? new Date(e.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
                  : ""}
              </time>
              <span className="tl-kind">{KINDS.find((k) => k.kind === e.kind)?.label}</span>
              <span className={e.kind === "tool" ? "tl-text mono" : "tl-text"}>{e.text}</span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
