import { useProjectOrder } from "./useProjectOrder";
import { useEffect, useMemo, useRef, useState, type ReactNode, type MouseEvent as ReactMouseEvent } from "react";
import { TruncatedText } from "./TruncatedText";
import { open } from "@tauri-apps/plugin-dialog";
import type { Folder, Session, SubAgent } from "./api";
import { AGENTS, needsAttention, relativeTime, sessionColor, tildify } from "./agents";
import { AnimeCard } from "./AnimeCard";
import { MusicCard } from "./MusicCard";
import { useContextMenu } from "./ContextMenu";
import { folderMenu, sessionMenu } from "./sessionMenu";
import { useStore } from "./store";
import { Dot, Icon, Monogram, StatusText } from "./ui";

/** Folder groups show this many sessions until expanded. */
const GROUP_PREVIEW = 8;
const COLLAPSED_KEY = "termlab.sidebar.collapsed";

type Filter = "all" | "attention" | "running";

/** Collapsed folders are a per-viewer convenience, kept in localStorage. */
function loadCollapsed(): Set<number> {
  try {
    return new Set(JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]") as number[]);
  } catch {
    return new Set();
  }
}

function saveCollapsed(ids: Set<number>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...ids]));
  } catch {
    // Storage unavailable: collapsing still works for this window.
  }
}

export function Sidebar() {
  const store = useStore();
  const { folders, sessions } = store;
  const order = useProjectOrder(folders, store.moveFolder);
  const menu = useContextMenu();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [collapsed, setCollapsed] = useState<Set<number>>(loadCollapsed);

  const counts: Record<Filter, number> = {
    all: sessions.length,
    attention: sessions.filter(needsAttention).length,
    running: sessions.filter((s) => s.status === "running").length,
  };

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const match = (s: Session) =>
      (filter === "all" || (filter === "attention" ? needsAttention(s) : s.status === "running")) &&
      (!q ||
        s.title.toLowerCase().includes(q) ||
        AGENTS[s.agent].name.toLowerCase().includes(q) ||
        (s.model ?? "").toLowerCase().includes(q));
    return folders.map((f) => ({
      folder: f,
      sessions: sessions.filter((s) => s.folderId === f.id && match(s)),
    }));
  }, [folders, sessions, query, filter]);

  const filtering = filter !== "all" || query.trim() !== "";

  // The sessions as listed, top to bottom (respecting the filter, collapsed folders and
  // "Show more"), for PgUp / PgDn.
  const ordered = useMemo(
    () =>
      groups.flatMap(({ folder, sessions: list }) => {
        if (collapsed.has(folder.id) && !filtering) return [];
        return expanded.has(folder.id) || filtering ? list : list.slice(0, GROUP_PREVIEW);
      }),
    [groups, collapsed, expanded, filtering],
  );
  const orderedRef = useRef(ordered);
  orderedRef.current = ordered;

  // PgUp / PgDn move to the previous / next session. Shift+PgUp/PgDn still scroll the
  // terminal's history. Not while a dialog or the palette is open.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key !== "PageUp" && e.key !== "PageDown") return;
      if (e.shiftKey || e.metaKey || e.ctrlKey || e.altKey) return;
      if (document.querySelector('[role="dialog"], [role="menu"]')) return;
      const list = orderedRef.current;
      if (list.length === 0) return;
      e.preventDefault();
      e.stopPropagation();
      const i = list.findIndex((x) => x.id === store.selectedId);
      const next =
        i === -1 ? 0 : e.key === "PageDown" ? Math.min(i + 1, list.length - 1) : Math.max(i - 1, 0);
      store.select(list[next].id, { view: "focus" });
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [store]);

  async function addFolder() {
    const path = await open({ directory: true, multiple: false, title: "Add project folder" });
    if (typeof path === "string") await store.addFolder(path);
  }

  function toggle(set: Set<number>, id: number): Set<number> {
    const next = new Set(set);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  }

  return (
    <nav className="sidebar" aria-label="Sessions">
      <div className="sidebar-top">
        <label htmlFor="sb-search" className="sr">
          Filter sessions
        </label>
        <div className="search-field">
          <Icon name="search" />
          <input
            id="sb-search"
            placeholder="Filter sessions"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <div role="group" aria-label="Show" className="filter-tabs">
          {(
            [
              ["all", "All"],
              ["attention", "Attention"],
              ["running", "Running"],
            ] as [Filter, string][]
          ).map(([key, label]) => (
            <button
              key={key}
              aria-pressed={filter === key}
              className={filter === key ? "on" : ""}
              onClick={() => setFilter(key)}
              title={key === "attention" ? "Needs you, or ended with an error" : undefined}
            >
              {label}
              <span className={`filter-count${key === "attention" && counts.attention > 0 ? " hot" : ""}`}>
                {counts[key]}
              </span>
            </button>
          ))}
        </div>
      </div>

      <div className="sidebar-list" ref={order.list}>
        <span className="sr" role="status">{order.announcement}</span>
        <div className="list-head">
          <span className="section-heading">Projects</span>
          <div className="spacer" />
          <button
            className="icon-btn small"
            aria-label="Load existing sessions"
            title="Load existing sessions…"
            onClick={() => store.openNewSession({ tab: "resume", everywhere: true })}
          >
            <Icon name="refresh" />
          </button>
          <button
            className="icon-btn small"
            aria-label="Add project folder"
            title="Add project folder…"
            onClick={() => void addFolder()}
          >
            <Icon name="folderPlus" />
          </button>
          <button
            className="icon-btn small"
            aria-label="New session"
            title="New session (⌘N)"
            onClick={() => store.openNewSession()}
          >
            <Icon name="plus" />
          </button>
        </div>

        {folders.length === 0 && (
          <div className="sidebar-empty">
            <p>No project folders yet.</p>
            <button className="btn small" onClick={() => void addFolder()}>
              <Icon name="folderPlus" /> Add a folder
            </button>
            <button className="btn small" onClick={() => store.openNewSession({ tab: "resume", everywhere: true })}>
              <Icon name="refresh" /> Load existing sessions
            </button>
          </div>
        )}

        {groups.map(({ folder, sessions: list }) => {
          if (filtering && list.length === 0) return null;
          const isCollapsed = collapsed.has(folder.id) && !filtering;
          const shown = expanded.has(folder.id) || filtering ? list : list.slice(0, GROUP_PREVIEW);
          return (
            <div key={folder.id} className={`folder-group${order.dragging === folder.id ? " is-dragging" : ""}${order.indicator?.id === folder.id ? (order.indicator.after ? " drop-after" : " drop-before") : ""}`}>
              <FolderHead
                dragHandle={order.handle(folder)}
                folder={folder}
                count={list.length}
                attention={list.filter(needsAttention).length}
                collapsed={isCollapsed}
                onToggle={() => {
                  const next = toggle(collapsed, folder.id);
                  setCollapsed(next);
                  saveCollapsed(next);
                }}
                onMenu={(e) => menu.open(e, `${folder.name} actions`, folderMenu(store, folder))}
              />
              {!isCollapsed && (
                <div className="folder-sessions" id={`folder-${folder.id}`}>
                  {shown.map((s) => (
                    <SessionRow
                      key={s.id}
                      session={s}
                      onMenu={(e) => menu.open(e, `${s.title} actions`, sessionMenu(store, s))}
                      onSubagentMenu={(e, a) => menu.open(e, `${a.name} actions`, [
                        {
                          label: "Open parent terminal",
                          onSelect: () => {
                            store.setSubagentView(null);
                            store.select(s.id, { view: "focus", tab: "terminal" });
                          },
                        },
                        {
                          label: "View sub-agent activity…",
                          onSelect: () => store.setSubagentView({ sessionId: s.id, subagentId: a.id }),
                        },
                      ])}
                    />
                  ))}
                  {!filtering && list.length > GROUP_PREVIEW && (
                    <button
                      className="show-more"
                      aria-expanded={expanded.has(folder.id)}
                      onClick={() => setExpanded(toggle(expanded, folder.id))}
                    >
                      {expanded.has(folder.id) ? "Show fewer" : `Show ${list.length - GROUP_PREVIEW} more`}
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {filtering && groups.every((g) => g.sessions.length === 0) && <p className="sidebar-note">No sessions match.</p>}
      </div>
      <AnimeCard />
      <MusicCard />
      {menu.element}
    </nav>
  );
}

function FolderHead({
  dragHandle,
  folder,
  count,
  attention,
  collapsed,
  onToggle,
  onMenu,
}: {
  dragHandle: ReactNode;
  folder: Folder;
  count: number;
  attention: number;
  collapsed: boolean;
  onToggle(): void;
  onMenu(e: ReactMouseEvent): void;
}) {
  const store = useStore();
  return (
    <div className="folder-head" data-folder-head={folder.id} onContextMenu={onMenu}>
      {dragHandle}
      <button
        className="folder-toggle"
        aria-expanded={!collapsed}
        aria-controls={`folder-${folder.id}`}
        title={tildify(folder.path, store.home)}
        onClick={onToggle}
      >
        <span className={`chevron${collapsed ? "" : " open"}`} aria-hidden="true">
          <Icon name="chevron" size={12} />
        </span>
        <TruncatedText className="folder-name" text={folder.name} />
        {attention > 0 && (
          <span className="folder-attention" title={`${attention} need${attention === 1 ? "s" : ""} attention`}>
            {attention}
          </span>
        )}
        <span className="folder-count">{count}</span>
      </button>
      <button
        className="icon-btn small folder-add"
        aria-label={`New session in ${folder.name}`}
        title="New session here"
        onClick={() => store.openNewSession({ folderId: folder.id })}
      >
        <Icon name="plus" />
      </button>
    </div>
  );
}

function SessionRow({ session: s, onMenu, onSubagentMenu }: {
  session: Session;
  onMenu(e: ReactMouseEvent): void;
  onSubagentMenu(e: ReactMouseEvent, agent: SubAgent): void;
}) {
  const store = useStore();
  const selected = s.id === store.selectedId && store.view === "focus";
  const ref = useRef<HTMLButtonElement>(null);
  // Keep the selected session visible (e.g. when moving with PgUp / PgDn).
  useEffect(() => {
    if (selected) ref.current?.scrollIntoView({ block: "nearest" });
  }, [selected]);
  // The daemon also retains recent history; the sidebar only shows active work.
  const subagents = s.live ? (s.subagents ?? []).filter((a) => a.running) : [];
  const tint = sessionColor(s.color);
  return (
    <>
    <button
      ref={ref}
      className={`session-row${selected ? " is-selected" : ""}${tint.className}`}
      style={tint.style}
      aria-current={selected ? "true" : undefined}
      aria-haspopup="menu"
      onClick={() => store.select(s.id, { view: "focus" })}
      onContextMenu={onMenu}
    >
      <Monogram agent={s.agent} size={24} />
      <span className="row-text">
        <TruncatedText className="row-title" text={s.title} />
        <span className="row-meta">
          <Dot status={s.status} />
          <StatusText status={s.status} />
        </span>
      </span>
      <span className="row-time">{relativeTime(s.lastActivityAt)}</span>
    </button>
    {subagents.length > 0 && (
      <ul className="subagents" aria-label={`Sub-agents of ${s.title}`}>
        {subagents.map((a) => (
          <li key={a.id}>
            <button
              className="subagent running"
              aria-haspopup="menu"
              title={`Open ${s.title}'s terminal · Right-click for sub-agent activity`}
              onClick={() => {
                store.setSubagentView(null);
                store.select(s.id, { view: "focus", tab: "terminal" });
              }}
              onContextMenu={(e) => onSubagentMenu(e, a)}
            >
              <span className="subagent-dot" aria-hidden="true" />
              <TruncatedText
                className="subagent-label"
                text={a.description ? `${a.name} · ${a.description}` : a.name}
              />
              <span className="subagent-time">
                <span className="sr">Working for </span>
                {a.startedAt ? relativeTime(a.startedAt) : ""}
              </span>
            </button>
          </li>
        ))}
      </ul>
    )}
    </>
  );
}
