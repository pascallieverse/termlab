import { useMemo, useState, type ReactNode } from "react";
import { api } from "./api";
import { AGENTS, STATUS, tildify } from "./agents";
import { Dialog } from "./Dialog";
import { useStore } from "./store";
import { Icon, Kbd, Monogram } from "./ui";

interface Item {
  id: string;
  group: string;
  label: string;
  /** Extra text that matches the query but isn't shown. */
  keywords?: string;
  kind: "session" | "command" | "folder";
  icon: ReactNode;
  right?: ReactNode;
  run(split: boolean): void;
}

/** Subsequence fuzzy match; higher is better, -1 means no match. */
export function fuzzyScore(query: string, text: string): number {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (!q) return 0;
  const direct = t.indexOf(q);
  if (direct >= 0) return 1000 - direct;
  let score = 0;
  let ti = 0;
  let streak = 0;
  for (const ch of q) {
    const found = t.indexOf(ch, ti);
    if (found < 0) return -1;
    streak = found === ti ? streak + 1 : 0;
    score += 10 + streak * 5 - Math.min(found - ti, 10);
    ti = found + 1;
  }
  return score;
}

export function Palette() {
  const store = useStore();
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const close = () => store.setPaletteOpen(false);
  const current = store.sessions.find((s) => s.id === store.selectedId);
  const defaultAgent = store.agents?.find((a) => a.installed);

  const items = useMemo(() => {
    const out: Item[] = [];
    for (const s of store.sessions) {
      const f = store.folderOf(s);
      out.push({
        id: `s:${s.id}`,
        group: `Sessions in ${f ? tildify(f.path, store.home) : "?"}`,
        label: s.title,
        keywords: `${AGENTS[s.agent].name} ${f?.name ?? ""} ${s.model ?? ""}`,
        kind: "session",
        icon: <Monogram agent={s.agent} size={26} />,
        right: (
          <span className="small" style={{ color: STATUS[s.status].text }}>
            {s.status === "needs_you" ? "▲ " : "● "}
            {STATUS[s.status].label}
          </span>
        ),
        run: (split) => {
          if (split) {
            if (store.selectedId && store.selectedId !== s.id) store.setSplit(s.id);
            else store.select(s.id, { view: "focus" });
          } else store.select(s.id, { view: "focus" });
        },
      });
    }
    if (current) {
      out.push({
        id: "c:handoff",
        group: "Actions",
        label: `Hand off “${current.title}” to another agent`,
        kind: "command",
        icon: <Icon name="arrow" size={16} />,
        right: <Kbd>⌘⇧H</Kbd>,
        run: () => store.setHandOffFor(current.id),
      });
    }
    for (const f of store.folders) {
      const path = tildify(f.path, store.home);
      if (defaultAgent) {
        out.push({
          id: `c:new:${f.id}`,
          group: "Actions",
          label: `New ${defaultAgent.name} session in ${path}`,
          kind: "command",
          icon: <Icon name="plus" size={16} />,
          right: <Kbd>⌘N</Kbd>,
          run: () => store.openNewSession({ folderId: f.id, agent: defaultAgent.agent }),
        });
      }
      out.push({
        id: `c:broadcast:${f.id}`,
        group: "Actions",
        label: `Broadcast a message to all ${f.name} sessions`,
        kind: "command",
        icon: <Icon name="lines" size={16} />,
        run: () => store.setBroadcastFor(f.id),
      });
      out.push({
        id: `c:stopidle:${f.id}`,
        group: "Actions",
        label: `Stop idle sessions in ${f.name}`,
        kind: "command",
        icon: <span style={{ color: "var(--error-text)" }}><Icon name="stop" size={16} /></span>,
        run: () =>
          void store.run(async () => {
            const n = await api.stopIdle(f.id);
            store.toast(`Stopped ${n} idle session${n === 1 ? "" : "s"} in ${f.name}`, "info");
          }),
      });
      out.push({
        id: `f:${f.id}`,
        group: "Folders",
        label: f.name,
        keywords: path,
        kind: "folder",
        icon: <Icon name="folder" size={16} />,
        right: <span className="mono faint small">{path}</span>,
        run: () => {
          const first = store.sessions.find((s) => s.folderId === f.id);
          if (first) store.select(first.id, { view: "focus" });
          else store.openNewSession({ folderId: f.id });
        },
      });
    }
    out.push(
      {
        id: "c:mission",
        group: "Actions",
        label: "Open mission control",
        kind: "command",
        icon: <Icon name="grid" size={16} />,
        run: () => store.setView("mission"),
      },
      {
        id: "c:resume",
        group: "Actions",
        label: "Resume a past session…",
        kind: "command",
        icon: <Icon name="refresh" size={16} />,
        run: () => store.openNewSession({ tab: "resume" }),
      },
      {
        id: "c:import",
        group: "Actions",
        label: "Load sessions from your agents (everywhere on this Mac)…",
        keywords: "import claude codex vibe history all existing attach background",
        kind: "command",
        icon: <Icon name="refresh" size={16} />,
        run: () => store.openNewSession({ tab: "resume", everywhere: true }),
      },
    );
    return out;
  }, [store, current, defaultAgent]);

  const results = useMemo(() => {
    let q = query;
    let kinds: Item["kind"][] = ["session", "command"];
    if (q.startsWith(">")) {
      kinds = ["command"];
      q = q.slice(1);
    } else if (q.startsWith("@")) {
      kinds = ["folder"];
      q = q.slice(1);
    }
    q = q.trim();
    const scored = items
      .filter((i) => kinds.includes(i.kind))
      .map((i) => {
        const raw = q ? Math.max(fuzzyScore(q, i.label), fuzzyScore(q, i.keywords ?? "") - 50) : 0;
        // Sessions outrank commands, so Enter on a fresh match opens rather than acts.
        return { i, score: raw < 0 ? -1 : raw + (i.kind === "session" ? 2000 : 0) };
      })
      .filter((x) => x.score >= 0);
    if (q) scored.sort((a, b) => b.score - a.score);
    // Keep groups together, in order of their best match.
    const order: string[] = [];
    for (const x of scored) if (!order.includes(x.i.group)) order.push(x.i.group);
    return order.flatMap((g) => scored.filter((x) => x.i.group === g).map((x) => x.i)).slice(0, 60);
  }, [items, query]);

  const active = Math.min(index, Math.max(0, results.length - 1));

  function choose(i: number, split: boolean) {
    const item = results[i];
    if (!item) return;
    close();
    item.run(split);
  }

  let lastGroup = "";
  return (
    <Dialog onClose={close} width={640} className="palette" title="Command palette">
      <div className="palette-input">
        <Icon name="search" size={18} />
        <label htmlFor="pal-q" className="sr">
          Search sessions and commands
        </label>
        <input
          id="pal-q"
          data-autofocus
          role="combobox"
          aria-expanded="true"
          aria-controls="pal-list"
          aria-activedescendant={results[active] ? `pal-${results[active].id}` : undefined}
          value={query}
          placeholder="Search sessions, or type > for commands, @ for folders"
          onChange={(e) => {
            setQuery(e.target.value);
            setIndex(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setIndex((i) => Math.min(i + 1, results.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setIndex((i) => Math.max(i - 1, 0));
            } else if (e.key === "Enter") {
              e.preventDefault();
              choose(active, e.metaKey || e.ctrlKey);
            }
          }}
        />
        <Kbd>esc</Kbd>
      </div>
      <div role="listbox" id="pal-list" aria-label="Results" className="palette-list">
        {results.length === 0 && <div className="pane-message">Nothing matches.</div>}
        {results.map((item, i) => {
          const header = item.group !== lastGroup ? item.group : null;
          lastGroup = item.group;
          return (
            <div key={item.id}>
              {header && <div className="palette-group">{header}</div>}
              <div
                id={`pal-${item.id}`}
                role="option"
                aria-selected={i === active}
                className={`palette-item${i === active ? " on" : ""}`}
                onMouseMove={() => setIndex(i)}
                onClick={(e) => choose(i, e.metaKey || e.ctrlKey)}
              >
                {item.icon}
                <span className="palette-label">{item.label}</span>
                {item.right}
                {i === active && <Kbd>↵</Kbd>}
              </div>
            </div>
          );
        })}
      </div>
      <div className="palette-foot">
        <span>↑↓ move</span>
        <span>↵ open</span>
        <span>⌘↵ open in split</span>
        <span className="push">Type &gt; for commands, @ for folders</span>
      </div>
    </Dialog>
  );
}
