import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  api,
  errorText,
  type AgentId,
  type Discovered,
  type ExternalSession,
  type PermissionPreset,
  type StartRequest,
} from "./api";
import { AGENTS, PERMISSIONS, defaultPermission, tildify, whenText } from "./agents";
import { Dialog } from "./Dialog";
import { FolderPicker } from "./FolderPicker";
import { useStore } from "./store";
import { Icon, Monogram } from "./ui";

const CUSTOM = "__custom__";

export function NewSessionDialog() {
  const store = useStore();
  const opts = store.newSession!;
  const [tab, setTab] = useState<"start" | "resume">(opts.tab ?? "start");
  const fixed = store.folders.find((f) => f.id === opts.folderId);

  return (
    <Dialog onClose={store.closeNewSession} className="new-session" labelledBy="ns-title">
      <div className="dialog-head">
        <div className="dialog-title">
          <h1 id="ns-title">New session</h1>
          {fixed && (
            <span className="mono faint small" title={fixed.path}>
              in {fixed.name} · {tildify(fixed.path, store.home)}
            </span>
          )}
        </div>
        <div role="tablist" aria-label="Mode" className="segmented dark">
          <button role="tab" aria-selected={tab === "start"} className={tab === "start" ? "on" : ""} onClick={() => setTab("start")}>
            Start new
          </button>
          <button role="tab" aria-selected={tab === "resume"} className={tab === "resume" ? "on" : ""} onClick={() => setTab("resume")}>
            Resume or attach
          </button>
        </div>
        <div className="spacer" />
        <button className="icon-btn" aria-label="Close" onClick={store.closeNewSession}>
          <Icon name="close" size={16} />
        </button>
      </div>
      {tab === "start" ? <StartNew /> : <ResumeTab />}
    </Dialog>
  );
}

function StartNew() {
  const store = useStore();
  const opts = store.newSession!;
  const agents = store.agents ?? [];
  const installed = agents.filter((a) => a.installed);
  const [agent, setAgent] = useState<AgentId | null>(opts.agent ?? installed[0]?.agent ?? null);
  // Starts on the folder of the session you're looking at; else the most recently used one.
  const [folderId, setFolderId] = useState<number | null>(() => {
    if (opts.folderId !== undefined) return opts.folderId;
    const current = store.sessions.find((x) => x.id === store.selectedId)?.folderId;
    if (current !== undefined && store.folders.some((f) => f.id === current)) return current;
    const latest = [...store.sessions].sort((a, b) => b.lastActivityAt - a.lastActivityAt)[0]?.folderId;
    return store.folders.find((f) => f.id === latest)?.id ?? store.folders[0]?.id ?? null;
  });
  const [modelChoice, setModelChoice] = useState("");
  const [customModel, setCustomModel] = useState("");
  const [permission, setPermission] = useState<PermissionPreset>(defaultPermission);
  const [notify, setNotify] = useState(true);
  const [starting, setStarting] = useState(false);
  // Opened from a folder's "+": that folder is fixed and the picker is hidden.
  const fixedFolder = opts.folderId !== undefined;

  const info = agents.find((a) => a.agent === agent);
  const model = modelChoice === CUSTOM ? customModel.trim() : modelChoice;

  useEffect(() => {
    if (!agent && installed[0]) setAgent(installed[0].agent);
  }, [agent, installed]);
  useEffect(() => {
    setModelChoice("");
    setCustomModel("");
  }, [agent]);

  const request: StartRequest | null = useMemo(
    () =>
      agent && folderId !== null
        ? {
            folderId,
            agent,
            model: info?.modelFlag && model ? model : null,
            permission,
            notify,
          }
        : null,
    [agent, folderId, info, model, permission, notify],
  );

  async function chooseFolder() {
    const path = await open({ directory: true, multiple: false, title: "Choose a project folder" });
    if (typeof path !== "string") return;
    const f = await store.addFolder(path);
    if (f) setFolderId(f.id);
  }

  async function start() {
    if (!request || starting) return;
    setStarting(true);
    const s = await store.run(() => api.startSession(request));
    setStarting(false);
    if (s) {
      store.closeNewSession();
      store.select(s.id, { view: "focus", tab: "terminal" });
    }
  }

  return (
    <>
      <div
        className="dialog-body"
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void start();
          }
        }}
      >
        <fieldset className="field">
          <legend className="label">Agent</legend>
          <div className="agent-cards">
            {agents.map((a) => (
              <button
                key={a.agent}
                className={`agent-card${agent === a.agent ? " on" : ""}`}
                aria-pressed={agent === a.agent}
                disabled={!a.installed}
                onClick={() => setAgent(a.agent)}
                title={a.installed ? `${a.installed.path}${a.verified ? "" : " · flags unverified"}` : `${a.binary} isn't on your PATH`}
              >
                <Monogram agent={a.agent} size={32} />
                <span className="agent-card-name">{a.name}</span>
                <span className="mono faint small">{a.installed ? a.binary : "Not installed"}</span>
              </button>
            ))}
          </div>
          {installed.length === 0 && store.agents && (
            <p className="muted small">No agent CLIs were found on your PATH.</p>
          )}
        </fieldset>

        {!fixedFolder && (
          <FolderPicker value={folderId} onChange={setFolderId} onChooseOther={() => void chooseFolder()} />
        )}

        <div className="two-col">
          <div className="field">
            <label htmlFor="ns-model" className="label">
              Model
            </label>
            {info?.modelFlag === false ? (
              <select id="ns-model" className="select" disabled>
                <option>Set in {info.name}'s own config</option>
              </select>
            ) : (
              <>
                <select id="ns-model" className="select" value={modelChoice} onChange={(e) => setModelChoice(e.target.value)}>
                  <option value="">Default (the agent's own setting)</option>
                  {info?.modelSuggestions.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                  <option value={CUSTOM}>Other…</option>
                </select>
                {modelChoice === CUSTOM && (
                  <input
                    className="input mono"
                    aria-label="Model name"
                    placeholder="model name"
                    value={customModel}
                    onChange={(e) => setCustomModel(e.target.value)}
                  />
                )}
              </>
            )}
          </div>
          <div className="field">
            <label htmlFor="ns-iso" className="label">
              Run in
            </label>
            <select id="ns-iso" className="select" defaultValue="checkout">
              <option value="checkout">Current checkout</option>
            </select>
          </div>
        </div>

        <fieldset className="field">
          <legend className="label">Permissions</legend>
          <div className="perm-cards">
            {PERMISSIONS.map((p) => (
              <button
                key={p.key}
                className={`perm-card${permission === p.key ? " on" : ""}`}
                aria-pressed={permission === p.key}
                onClick={() => setPermission(p.key)}
              >
                <span className="perm-label">{p.label}</span>
                <span className="muted small">{p.desc}</span>
              </button>
            ))}
          </div>
        </fieldset>

      </div>

      <div className="dialog-foot">
        <label className="check">
          <input type="checkbox" checked={notify} onChange={(e) => setNotify(e.target.checked)} />
          Notify me when it needs input
        </label>
        <div className="spacer" />
        <button className="btn tall" onClick={store.closeNewSession}>
          Cancel
        </button>
        <button className="btn primary tall" disabled={!request || starting} onClick={() => void start()}>
          Start session <span className="mono kbd-hint">⌘↵</span>
        </button>
      </div>
    </>
  );
}

type Scope = "folders" | "everywhere";

function ResumeTab() {
  const store = useStore();
  const opts = store.newSession!;
  // Opened for one folder: only that folder's sessions, no scope switch.
  const fixedFolder = opts.folderId ?? null;
  const [scope, setScope] = useState<Scope>(opts.everywhere ? "everywhere" : "folders");
  // Past sessions are opt-in, except from "Load existing sessions", where they're the point.
  const [showPast, setShowPast] = useState(!!opts.everywhere);
  const [agentFilter, setAgentFilter] = useState<AgentId | "all">("all");
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<Discovered[] | null>(null);
  const [external, setExternal] = useState<ExternalSession[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [version, setVersion] = useState(0);
  const live = store.sessions.filter((s) => s.live && (fixedFolder === null || s.folderId === fixedFolder));

  useEffect(() => {
    api.external().then(setExternal, () => setExternal([]));
  }, [version]);

  useEffect(() => {
    setFound(null);
    setSelected(new Set());
    setError(null);
    if (!showPast) return;
    api
      .discover(fixedFolder, fixedFolder === null && scope === "everywhere")
      .then(setFound, (e) => setError(errorText(e)));
  }, [scope, version, showPast, fixedFolder]);

  const key = (d: { agent: AgentId; agentSessionId: string }) => `${d.agent}:${d.agentSessionId}`;
  const q = query.trim().toLowerCase();
  const visible = (found ?? []).filter(
    (d) =>
      (agentFilter === "all" || d.agent === agentFilter) &&
      (!q || d.title.toLowerCase().includes(q) || d.cwd.toLowerCase().includes(q)),
  );
  // Rows that can still be imported (not already in Termlab).
  const importable = visible.filter((d) => !d.termlabSession);
  const chosen = importable.filter((d) => selected.has(key(d)));
  const agentsPresent = Array.from(new Set((found ?? []).map((d) => d.agent)));

  function openSession(id: string) {
    store.closeNewSession();
    store.select(id, { view: "focus", tab: "terminal" });
  }

  async function doImport(items: Discovered[]) {
    if (items.length === 0) return;
    setBusy(true);
    const r = await store.run(() =>
      api.importSessions(
        items.map(({ agent, agentSessionId, title, cwd, lastActivityAt, path }) => ({
          agent,
          agentSessionId,
          title,
          cwd,
          lastActivityAt,
          path,
        })),
      ),
    );
    setBusy(false);
    if (r) {
      const parts = [`Imported ${r.imported} session${r.imported === 1 ? "" : "s"}`];
      if (r.foldersAdded) parts.push(`added ${r.foldersAdded} folder${r.foldersAdded === 1 ? "" : "s"}`);
      if (r.skipped) parts.push(`skipped ${r.skipped} whose folder no longer exists`);
      store.toast(parts.join(", "), "info");
      setVersion((v) => v + 1);
    }
  }

  async function resume(d: Discovered) {
    let folderId = d.folderId;
    if (folderId === null) {
      const f = await store.addFolder(d.cwd);
      if (!f) return;
      folderId = f.id;
    }
    const s = await store.run(() =>
      api.startSession({
        folderId: folderId!,
        agent: d.agent,
        permission: defaultPermission(),
        title: d.title,
        mode: { kind: "resume", agentSessionId: d.agentSessionId },
      }),
    );
    if (s) openSession(s.id);
  }

  async function restart(id: string) {
    const s = await store.run(() => api.restart(id));
    if (s) openSession(s.id);
  }

  async function attach(e: ExternalSession) {
    const s = await store.run(() => api.attachExternal(e.agent, e.agentSessionId));
    if (s) openSession(s.id);
  }

  const elsewhere = (external ?? []).filter(
    (e) => !e.termlabSession && (fixedFolder === null || e.folderId === fixedFolder),
  );

  return (
    <div className="dialog-body">
      <div className="resume-toolbar">
        {fixedFolder === null && (
        <div role="radiogroup" aria-label="Where to look" className="segmented dark">
          <button role="radio" aria-checked={scope === "folders"} className={scope === "folders" ? "on" : ""} onClick={() => setScope("folders")}>
            Project folders
          </button>
          <button role="radio" aria-checked={scope === "everywhere"} className={scope === "everywhere" ? "on" : ""} onClick={() => setScope("everywhere")}>
            Everywhere on this Mac
          </button>
        </div>
        )}
        <label htmlFor="resume-q" className="sr">
          Filter sessions
        </label>
        <input
          id="resume-q"
          className="input"
          placeholder="Filter by title or folder"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <label htmlFor="resume-agent" className="sr">
          Agent
        </label>
        <select id="resume-agent" className="select compact" value={agentFilter} onChange={(e) => setAgentFilter(e.target.value as AgentId | "all")}>
          <option value="all">All agents</option>
          {agentsPresent.map((a) => (
            <option key={a} value={a}>
              {AGENTS[a].name}
            </option>
          ))}
        </select>
      </div>

      {live.length === 0 && elsewhere.length === 0 && external !== null && (
        <p className="muted small">
          Nothing is running {fixedFolder === null ? "right now" : "in this folder right now"}.
        </p>
      )}
      {(live.length > 0 || elsewhere.length > 0) && (
        <section className="resume-section" aria-label="Running now">
          <h2 className="section-heading">Running now</h2>
          {live.map((s) => {
            const f = store.folderOf(s);
            return (
              <div key={s.id} className="found-row">
                <Monogram agent={s.agent} size={32} />
                <div className="found-text">
                  <span className="found-title">{s.title}</span>
                  <span className="mono faint small">{f ? tildify(f.path, store.home) : ""} · in Termlab</span>
                </div>
                <span className="tag live">Live</span>
                <button className="btn" onClick={() => openSession(s.id)}>
                  Open
                </button>
              </div>
            );
          })}
          {elsewhere.map((e) => (
            <div key={key(e)} className="found-row">
              <Monogram agent={e.agent} size={32} />
              <div className="found-text">
                <span className="found-title">{e.name ?? `${AGENTS[e.agent].name} session`}</span>
                <span className="mono faint small">
                  {tildify(e.cwd, store.home)} ·{" "}
                  {e.kind === "background" ? `${AGENTS[e.agent].name} background session` : "open in another terminal"}
                  {e.status === "needs_you" ? " · waiting for input" : e.status === "running" ? " · working" : ""}
                </span>
              </div>
              <span className="tag live">{e.kind === "background" ? "Background" : "Elsewhere"}</span>
              {e.attachable ? (
                <button className="btn" onClick={() => void attach(e)} title={`Runs ${AGENTS[e.agent].name}'s attach command in a Termlab terminal; the session keeps running when you close it`}>
                  Attach
                </button>
              ) : (
                <button className="btn" disabled title="Sessions open in another terminal window can't be attached to">
                  Attach
                </button>
              )}
            </div>
          ))}
        </section>
      )}

      <label className="check">
        <input type="checkbox" checked={showPast} onChange={(e) => setShowPast(e.target.checked)} />
        Show past sessions
      </label>

      {showPast && (
      <section className="resume-section" aria-label="Past sessions">
        <div className="resume-section-head">
          <h2 className="section-heading">
            Past sessions{found ? ` · ${visible.length}` : ""}
          </h2>
          <div className="spacer" />
          {importable.length > 0 && (
            <>
              <label className="check small">
                <input
                  type="checkbox"
                  checked={chosen.length === importable.length}
                  onChange={(e) =>
                    setSelected(e.target.checked ? new Set(importable.map(key)) : new Set())
                  }
                />
                Select all
              </label>
              <button className="btn small" disabled={busy || chosen.length === 0} onClick={() => void doImport(chosen)}>
                Import selected ({chosen.length})
              </button>
              <button className="btn small primary" disabled={busy} onClick={() => void doImport(importable)}>
                Import all ({importable.length})
              </button>
            </>
          )}
        </div>
        <p className="muted small">
          {scope === "everywhere"
            ? "Every session Claude Code, Codex and Mistral Vibe recorded on this Mac. Importing adds them (and their folders) to Termlab, ready to resume."
            : "Sessions the agents recorded in your project folders."}
        </p>
        {error && <p className="error-text">{error}</p>}
        {found === null && !error && <p className="muted small">Looking for past sessions…</p>}
        {found !== null && visible.length === 0 && (
          <p className="muted small">
            {scope === "folders" ? "None in your project folders. Try “Everywhere on this Mac”." : "No past sessions found."}
          </p>
        )}
        {visible.map((d) => {
          const owned = d.termlabSession ? store.sessions.find((s) => s.id === d.termlabSession) : undefined;
          if (owned?.live) return null;
          const k = key(d);
          return (
            <div key={k} className="found-row">
              {!owned ? (
                <input
                  type="checkbox"
                  className="row-check"
                  aria-label={`Select “${d.title}”`}
                  checked={selected.has(k)}
                  onChange={(e) => {
                    const next = new Set(selected);
                    if (e.target.checked) next.add(k);
                    else next.delete(k);
                    setSelected(next);
                  }}
                />
              ) : (
                <span className="row-check" aria-hidden="true" />
              )}
              <Monogram agent={d.agent} size={32} />
              <div className="found-text">
                <span className="found-title">{d.title}</span>
                <span className="mono faint small">
                  {tildify(d.cwd, store.home)} · {whenText(d.lastActivityAt)}
                </span>
              </div>
              <span className={`tag${d.runningElsewhere ? " live" : ""}`}>
                {d.runningElsewhere ? "Running" : owned ? "In Termlab" : "Past"}
              </span>
              {owned ? (
                <>
                  <button className="btn" onClick={() => openSession(owned.id)}>
                    Open
                  </button>
                  <button
                    className="btn"
                    disabled={d.runningElsewhere}
                    title={d.runningElsewhere ? "Running right now; attach to it above instead" : "Continue it in its Termlab session"}
                    onClick={() => void restart(owned.id)}
                  >
                    Resume here
                  </button>
                </>
              ) : (
                <>
                  <button className="btn" disabled={busy} onClick={() => void doImport([d])} title="Add to Termlab without starting it">
                    Import
                  </button>
                  <button
                    className="btn"
                    disabled={d.runningElsewhere}
                    title={d.runningElsewhere ? "Running right now; attach to it above instead" : `Resume this ${AGENTS[d.agent].name} session in Termlab`}
                    onClick={() => void resume(d)}
                  >
                    Resume
                  </button>
                </>
              )}
            </div>
          );
        })}
      </section>
      )}
    </div>
  );
}
