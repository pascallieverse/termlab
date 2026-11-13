import { useCallback, useEffect, useId, useRef, useState } from "react";
import { AGENTS, AGENT_ORDER, relativeTime, whenText } from "./agents";
import { AgentLogo } from "./AgentLogo";
import { api, type AgentId, type PlanUsage, type UsageWindow } from "./api";

// The subscription plans, in the top bar: for each agent, whether it's connected, its plan,
// and how much of each limit is used where the service reports that. The agents themselves
// are asked: Claude Code answers a usage request (and reports through its status line),
// Codex's app server answers a rate-limit request, Vibe's own account cache gives its plan,
// and Antigravity says whether it's signed in.

const POLL_MS = 20_000;
/** Older than this, the numbers are shown as possibly out of date. */
const STALE_MS = 30 * 60_000;

const LABELS: Record<string, string> = {
  five_hour: "Current session (5 hours)",
  seven_day: "Current week (all models)",
  seven_day_opus: "Current week (Opus)",
  seven_day_sonnet: "Current week (Sonnet)",
  spend_limit: "Spend limit",
};

/** Where a service shows usage itself, for agents whose numbers Termlab can't get. */
const USAGE_PAGE: Partial<Record<AgentId, { url: string; label: string }>> = {
  vibe: { url: "https://console.mistral.ai/codestral/cli", label: "View usage on Mistral" },
};

function label(w: UsageWindow, agent: AgentId): string {
  // "All models" is Claude's distinction (it also has per-model weeks).
  if (w.id === "seven_day" && agent !== "claude") return "Current week";
  return LABELS[w.id] ?? w.label ?? w.id.replace(/[_:]/g, " ");
}

function level(percent: number): "ok" | "warn" | "full" {
  return percent >= 95 ? "full" : percent >= 80 ? "warn" : "ok";
}

export function UsageChip() {
  const [usage, setUsage] = useState<PlanUsage[]>([]);
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  const load = useCallback(
    () =>
      api.planUsage().then(setUsage, () => {
        // An older background service doesn't know this request.
      }),
    [],
  );

  useEffect(() => {
    void load();
    // Ask the agents for fresh numbers when the app starts.
    void api.refreshPlanUsage().then(setUsage, () => {});
    const timer = setInterval(load, POLL_MS);
    window.addEventListener("focus", load);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", load);
    };
  }, [load]);

  useEffect(() => {
    if (!open) return;
    // Opening the panel asks for fresh numbers; they arrive a moment later.
    void api.refreshPlanUsage().then(setUsage, () => {});
    const soon = [setTimeout(load, 2500), setTimeout(load, 6000), setTimeout(load, 12_000)];
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      soon.forEach(clearTimeout);
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, load]);

  const agents = AGENT_ORDER.map((id) => usage.find((u) => u.agent === id)).filter(
    (u): u is PlanUsage => u !== undefined,
  );
  if (agents.length === 0) return null;

  const measured = agents.filter((u) => u.windows.length > 0);
  const worst = Math.max(0, ...measured.flatMap((u) => u.windows.map((w) => w.usedPercentage)));

  return (
    <div className="usage" ref={root}>
      <button
        className={`hchip usage-chip ${level(worst)}`}
        aria-expanded={open}
        aria-haspopup="true"
        aria-label="Plan usage"
        title="Plan usage"
        onClick={() => setOpen((o) => !o)}
      >
        {measured.length === 0 && <span>Plans</span>}
        {measured.map((u) => (
          <span
            key={u.agent}
            className="usage-chip-agent"
            title={`${AGENTS[u.agent].name}: ${u.windows
              .map((w) => `${label(w, u.agent)} ${Math.round(w.usedPercentage)}%`)
              .join(", ")}`}
          >
            <AgentLogo agent={u.agent} size={13} />
            {u.windows.map((w, i) => (
              <span key={w.id}>
                {i > 0 && <span aria-hidden="true"> · </span>}
                <strong className={level(w.usedPercentage)}>{Math.round(w.usedPercentage)}%</strong>
              </span>
            ))}
          </span>
        ))}
      </button>
      {open && (
        <div className="usage-panel" role="group" aria-label="Plan usage">
          <div className="usage-panel-head">
            <strong>Plan usage</strong>
            <UsageInfo />
          </div>
          {agents.map((u) => (
            <AgentUsage key={u.agent} usage={u} />
          ))}
        </div>
      )}
    </div>
  );
}

function UsageInfo() {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const id = useId();

  useEffect(() => {
    if (!open) return;
    const closeOutside = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", closeOutside);
    return () => window.removeEventListener("mousedown", closeOutside);
  }, [open]);

  return (
    <div
      className="usage-info"
      ref={root}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget)) setOpen(false);
      }}
      onKeyDown={(e) => {
        if (open && e.key === "Escape") {
          e.stopPropagation();
          setOpen(false);
        }
      }}
    >
      <button
        className="usage-info-button"
        aria-label="How plan usage is updated"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-describedby={open ? id : undefined}
        onClick={() => setOpen((value) => !value)}
      >
        <span aria-hidden="true">i</span>
      </button>
      {open && (
        <div id={id} role="tooltip" className="usage-info-popup">
          Reported by the agents themselves: each is asked when you open this, and every few minutes while one
          of its sessions runs.
        </div>
      )}
    </div>
  );
}

function AgentUsage({ usage: u }: { usage: PlanUsage }) {
  const age = relativeTime(u.updatedAt);
  const stale = u.windows.length > 0 && Date.now() - u.updatedAt > STALE_MS;
  const page = USAGE_PAGE[u.agent];
  const state = u.connected === false ? "Not signed in" : u.connected ? "Connected" : null;
  return (
    <section className="usage-agent-group" aria-label={AGENTS[u.agent].name}>
      <div className="usage-agent-head">
        <span className="usage-agent-name">
          <AgentLogo agent={u.agent} size={15} />
          <strong>{AGENTS[u.agent].name}</strong>
          {u.plan && <span className="usage-plan">{u.plan}</span>}
        </span>
        <span className={`small ${u.connected === false ? "usage-off" : "faint"}`}>
          {state}
          {state && u.windows.length > 0 ? " · " : ""}
          {u.windows.length > 0 ? (age === "now" ? "just now" : `${age} ago`) : ""}
          {stale ? " · may be out of date" : ""}
        </span>
      </div>
      {u.windows.map((w) => (
        <WindowBar key={w.id} window={w} agent={u.agent} />
      ))}
      {u.windows.length === 0 && u.note && <p className="muted small usage-agent-note">{u.note}</p>}
      {u.windows.length === 0 && u.connected !== false && page && (
        <button className="link-btn small usage-link" onClick={() => void api.openExternal(page.url).catch(() => {})}>
          {page.label} ↗
        </button>
      )}
    </section>
  );
}

function WindowBar({ window: w, agent }: { window: UsageWindow; agent: AgentId }) {
  const percent = Math.round(w.usedPercentage);
  const name = label(w, agent);
  return (
    <div className="usage-window">
      <div className="usage-window-head">
        <span>{name}</span>
        <span className="usage-percent">{percent}% used</span>
      </div>
      <div
        className={`usage-bar ${level(w.usedPercentage)}`}
        role="progressbar"
        aria-label={name}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.min(100, percent)}
      >
        <span style={{ width: `${Math.min(100, Math.max(0, w.usedPercentage))}%` }} />
      </div>
      {w.resetsAt && <div className="faint small">Resets {whenText(w.resetsAt)}</div>}
    </div>
  );
}
