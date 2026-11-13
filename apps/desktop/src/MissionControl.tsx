import { useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import { TruncatedText } from "./TruncatedText";
import { api, type Session } from "./api";
import { AGENTS, AGENT_ORDER, STATUS, formatCost, needsAttention, relativeTime, sessionColor, tildify } from "./agents";
import { useContextMenu } from "./ContextMenu";
import { sessionMenu } from "./sessionMenu";
import { useStore } from "./store";
import { Icon, Monogram, StatusPill } from "./ui";

type GroupBy = "folder" | "agent" | "status";

const STATUS_ORDER: Session["status"][] = ["needs_you", "error", "running", "idle", "done"];

/** All sessions as tiles with live previews. */
export function MissionControl() {
  const store = useStore();
  const { sessions, folders, home } = store;
  const [groupBy, setGroupBy] = useState<GroupBy>("folder");
  const waiting = sessions.filter(needsAttention);
  const menu = useContextMenu();

  const groups = useMemo(() => {
    if (groupBy === "folder") {
      return folders.map((f) => ({
        key: `f${f.id}`,
        title: f.name,
        sub: tildify(f.path, home),
        folderId: f.id as number | null,
        list: sessions.filter((s) => s.folderId === f.id),
      }));
    }
    if (groupBy === "agent") {
      return AGENT_ORDER.map((a) => ({
        key: a,
        title: AGENTS[a].name,
        sub: "",
        folderId: null,
        list: sessions.filter((s) => s.agent === a),
      })).filter((g) => g.list.length > 0);
    }
    return STATUS_ORDER.map((st) => ({
      key: st,
      title: STATUS[st].label,
      sub: "",
      folderId: null,
      list: sessions.filter((s) => s.status === st),
    })).filter((g) => g.list.length > 0);
  }, [groupBy, folders, sessions, home]);

  return (
    <div className="mission">
      <div className="mission-bar">
        <div role="group" aria-label="Group by" className="groupby">
          Group by
          {(["folder", "agent", "status"] as GroupBy[]).map((g) => (
            <button
              key={g}
              className={`chip${groupBy === g ? " on" : ""}`}
              aria-pressed={groupBy === g}
              onClick={() => setGroupBy(g)}
            >
              {g[0].toUpperCase() + g.slice(1)}
            </button>
          ))}
        </div>
      </div>

      {waiting.length > 0 && (
        <section aria-label="Needs you" className="waiting-banner">
          <span className="waiting-title">
            ▲ {waiting.length} session{waiting.length === 1 ? " is" : "s are"} waiting on you
          </span>
          {waiting.slice(0, 3).map((s) => (
            <span key={s.id} className="waiting-item">
              <b>{store.folderOf(s)?.name ?? ""}</b> · {AGENTS[s.agent].name}{" "}
              {s.status === "error" ? "exited with an error" : (s.approval?.detail ?? "needs your input")}
            </span>
          ))}
          <div className="spacer" />
          <button className="btn amber-outline" onClick={() => store.select(waiting[0].id, { view: "focus", tab: "terminal" })}>
            Review one by one
          </button>
        </section>
      )}

      {sessions.length === 0 && (
        <div className="pane-message">No sessions yet. Start one with ⌘N.</div>
      )}

      {groups.map((g) =>
        groupBy === "folder" && g.list.length === 0 ? null : (
          <section key={g.key} className="mc-group">
            <div className="mc-group-head">
              <Icon name="folder" size={15} />
              <h2>{g.title}</h2>
              {g.sub && <span className="mono faint">{g.sub}</span>}
              <span className="muted small">· {summary(g.list)}</span>
              <div className="spacer" />
              {g.folderId !== null && (
                <button className="btn small" onClick={() => store.openNewSession({ folderId: g.folderId! })}>
                  + Session here
                </button>
              )}
            </div>
            <div className="tiles">
              {g.list.map((s) => (
                <Tile key={s.id} session={s} onMenu={(e) => menu.open(e, `${s.title} actions`, sessionMenu(store, s))} />
              ))}
            </div>
          </section>
        ),
      )}
      {menu.element}
    </div>
  );
}

function Tile({ session: s, onMenu }: { session: Session; onMenu(e: ReactMouseEvent): void }) {
  const store = useStore();
  const hot = needsAttention(s);
  const cost = s.billing === "api_key" && s.usage?.costUsd != null ? formatCost(s.usage.costUsd) : null;
  return (
    <article
      className={`tile${sessionColor(s.color).className}`}
      style={{ borderColor: hot ? STATUS[s.status].color + "88" : undefined, ...sessionColor(s.color).style }}
      onContextMenu={onMenu}
    >
      <div className="tile-head">
        <Monogram agent={s.agent} size={32} />
        <div className="tile-title">
          <TruncatedText as="h3" text={s.title} />
          <span className="muted small">
            {AGENTS[s.agent].name}
            {s.model ? <span className="mono"> · {s.model}</span> : null}
          </span>
        </div>
        <StatusPill session={s} compact />
      </div>
      <div className="tile-preview mono" aria-label="Last terminal lines">
        {(s.preview.length ? s.preview : [s.live ? "" : "Session ended."]).map((line, i) => (
          <div key={i}>{line || " "}</div>
        ))}
      </div>
      <div className="tile-foot">
        <span>ctx {s.contextPercent ?? "—"}%</span>
        {cost && <span>{cost}</span>}
        <span>{relativeTime(s.lastActivityAt)}</span>
        <div className="spacer" />
        {s.status === "needs_you" && s.approval?.options.some((o) => o.choice === "allow_once") && (
          <button className="btn primary small" onClick={() => void store.run(() => api.answer(s.id, "allow_once"))}>
            Allow
          </button>
        )}
        <button className="btn small" onClick={() => store.select(s.id, { view: "focus", tab: "terminal" })}>
          Open
        </button>
      </div>
    </article>
  );
}

function summary(list: Session[]): string {
  const n = (st: Session["status"]) => list.filter((s) => s.status === st).length;
  const parts: string[] = [];
  if (n("running")) parts.push(`${n("running")} running`);
  if (n("needs_you") + n("error")) parts.push(`${n("needs_you") + n("error")} need attention`);
  if (n("idle")) parts.push(`${n("idle")} idle`);
  if (n("done")) parts.push(`${n("done")} done`);
  return parts.join(" · ") || "no sessions";
}
