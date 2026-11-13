import { useEffect, useState } from "react";
import { PERMISSIONS, defaultPermission, setDefaultPermission } from "./agents";
import type { PermissionPreset } from "./api";
import { TimelineView } from "./TimelineView";
import { animeArtEnabled, setAnimeArtEnabled } from "./AnimeCard";
import { musicEnabled, setMusicEnabled } from "./MusicCard";
import { THEMES, chooseTheme, currentTheme, type ThemeId } from "./themes";
import { api } from "./api";
import { Dialog } from "./Dialog";
import { useStore } from "./store";
import { Icon } from "./ui";

export function RenameDialog() {
  const store = useStore();
  const s = store.sessions.find((x) => x.id === store.renameFor);
  const [title, setTitle] = useState(s?.title ?? "");
  const close = () => store.setRenameFor(null);
  if (!s) return null;
  async function save() {
    if (!s || !title.trim()) return;
    const ok = await store.run(() => api.setTitle(s.id, title));
    if (ok !== undefined) close();
  }
  return (
    <Dialog onClose={close} width={480} labelledBy="rn-title">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <div className="dialog-head">
          <h1 id="rn-title">Rename session</h1>
        </div>
        <div className="dialog-body">
          <label htmlFor="rn-input" className="label">
            Title
          </label>
          <input id="rn-input" data-autofocus className="input" value={title} onChange={(e) => setTitle(e.target.value)} />
        </div>
        <div className="dialog-foot">
          <div className="spacer" />
          <button type="button" className="btn tall" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="btn primary tall" disabled={!title.trim()}>
            Rename
          </button>
        </div>
      </form>
    </Dialog>
  );
}

export function ConfirmDialog() {
  const store = useStore();
  const req = store.confirmRequest;
  if (!req) return null;
  const close = () => store.confirm(null);
  return (
    <Dialog onClose={close} width={460} labelledBy="cf-title">
      <div className="dialog-head">
        <h1 id="cf-title">{req.title}</h1>
      </div>
      <div className="dialog-body">
        <p className="muted">{req.body}</p>
      </div>
      <div className="dialog-foot">
        <div className="spacer" />
        <button className="btn tall" data-autofocus onClick={close}>
          Cancel
        </button>
        <button
          className="btn danger tall"
          onClick={() => {
            close();
            req.onConfirm();
          }}
        >
          {req.action}
        </button>
      </div>
    </Dialog>
  );
}

/** App settings: detected agents and the background service. */
export function SettingsDialog() {
  const store = useStore();
  const close = () => store.setSettingsOpen(false);
  const [checking, setChecking] = useState(false);
  return (
    <Dialog onClose={close} width={520} labelledBy="st-title">
      <div className="dialog-head">
        <h1 id="st-title">Settings</h1>
        <div className="spacer" />
        <button className="icon-btn" aria-label="Close" onClick={close}>
          <Icon name="close" size={16} />
        </button>
      </div>
      <div className="dialog-body">
        <section className="settings-section">
          <h2 className="section-heading">Detected agents</h2>
          {store.agents === null ? (
            <p className="muted small">Looking…</p>
          ) : (
            <dl className="agent-list">
              {store.agents.map((a) => (
                <div key={a.agent} className="agent-list-row">
                  <dt>{a.name}</dt>
                  <dd className="mono" title={a.installed?.path ?? `${a.binary} isn't on your PATH`}>
                    {a.installed ? (a.installed.version ?? "installed") : "not installed"}
                  </dd>
                  <dd className="muted small">
                    {a.installed
                      ? a.installed.loginBilling === "subscription"
                        ? "Signed in with a subscription"
                        : a.installed.loginBilling === "api_key"
                          ? "Signed in with an API key"
                          : a.binary
                      : `${a.binary} not found on PATH`}
                  </dd>
                </div>
              ))}
            </dl>
          )}
          <div>
            <button
              className="btn"
              disabled={checking}
              onClick={async () => {
                setChecking(true);
                await store.refreshAgents();
                setChecking(false);
              }}
            >
              <Icon name="refresh" /> {checking ? "Checking…" : "Check again"}
            </button>
          </div>
        </section>
        <section className="settings-section">
          <h2 className="section-heading">New sessions</h2>
          <DefaultPermissionPicker />
        </section>
        <section className="settings-section">
          <h2 className="section-heading">Appearance</h2>
          <ThemePicker />
          <MusicToggle />
          <AnimeToggle />
        </section>
        <section className="settings-section">
          <h2 className="section-heading">Background service</h2>
          <p className="muted small">
            {store.daemon.connected
              ? "Connected. Sessions keep running when you close Termlab."
              : (store.daemon.message ?? "Not connected.")}
          </p>
        </section>
      </div>
    </Dialog>
  );
}

function MusicToggle() {
  const [on, setOn] = useState(musicEnabled);
  return (
    <label className="check">
      <input
        type="checkbox"
        checked={on}
        onChange={(e) => {
          setOn(e.target.checked);
          setMusicEnabled(e.target.checked);
        }}
      />
      Show the YouTube Music player in the sidebar
    </label>
  );
}

function AnimeToggle() {
  const [on, setOn] = useState(animeArtEnabled);
  return (
    <label className="check">
      <input
        type="checkbox"
        checked={on}
        onChange={(e) => {
          setOn(e.target.checked);
          setAnimeArtEnabled(e.target.checked);
        }}
      />
      Show random anime art in the sidebar (from nekos.best)
    </label>
  );
}

const SWATCH_KEYS = ["red", "green", "yellow", "blue", "magenta", "cyan"] as const;

/** One choice for the app and its terminals; each preview shows a sidebar beside a terminal. */
function ThemePicker() {
  const [current, setCurrent] = useState<ThemeId>(currentTheme().id);
  return (
    <>
      <div role="radiogroup" aria-label="Theme" className="theme-grid">
        {THEMES.map((t) => (
          <button
            key={t.id}
            role="radio"
            aria-checked={current === t.id}
            className={`theme-option${current === t.id ? " on" : ""}`}
            onClick={() => {
              chooseTheme(t.id);
              setCurrent(t.id);
            }}
          >
            <span
              className="theme-swatch"
              aria-hidden="true"
              style={{ background: t.tokens.bg, borderColor: t.tokens["border-strong"] }}
            >
              <span className="theme-swatch-side" style={{ background: t.tokens.panel }}>
                <span style={{ background: t.tokens.selected }} />
              </span>
              <span className="theme-swatch-term mono" style={{ background: t.tokens.terminal, color: t.tokens.text }}>
                <span>
                  <span style={{ color: t.ansi.green }}>❯</span> npm test
                </span>
                <span className="term-swatch-colors">
                  {SWATCH_KEYS.map((k) => (
                    <span key={k} style={{ background: t.ansi[k] }} />
                  ))}
                </span>
              </span>
            </span>
            <span className="theme-name">{t.name}</span>
            <span className="muted small">{t.description}</span>
          </button>
        ))}
      </div>
      <p className="muted small">
        A theme colours the app and its terminals together. Agents that draw with their own exact colours
        (Claude Code's default themes do) keep those for that text.
      </p>
    </>
  );
}

/** What a sub-agent did: its task, tool calls and messages, from the agent's own record. */
export function SubagentDialog() {
  const store = useStore();
  const view = store.subagentView;
  const session = store.sessions.find((x) => x.id === view?.sessionId);
  const agent = session?.subagents?.find((a) => a.id === view?.subagentId);
  // Keep showing a sub-agent that has dropped off the list (finished a while ago).
  const [last, setLast] = useState(agent);
  useEffect(() => {
    if (agent) setLast(agent);
  }, [agent]);
  const shown = agent ?? last;
  const close = () => store.setSubagentView(null);
  if (!view || !session) return null;
  return (
    <Dialog onClose={close} width={760} labelledBy="sub-title">
      <div className="dialog-head">
        <div className="subagent-head">
          <h1 id="sub-title">{shown?.name ?? "Sub-agent"}</h1>
          <span className="muted small">
            {shown?.description ? `${shown.description} · ` : ""}
            {shown?.model ? `${shown.model} · ` : ""}
            {shown?.running ? "working" : "finished"} · in {session.title}
          </span>
        </div>
        <div className="spacer" />
        <button className="icon-btn" aria-label="Close" onClick={close}>
          <Icon name="close" />
        </button>
      </div>
      <div className="subagent-timeline">
        <TimelineView
          sessionId={view.sessionId}
          subagentId={view.subagentId}
          status={shown?.running ? "running" : "done"}
        />
      </div>
    </Dialog>
  );
}

/** Which permissions the New session dialog starts on. Each session can still choose its own. */
function DefaultPermissionPicker() {
  const [current, setCurrent] = useState<PermissionPreset>(defaultPermission);
  return (
    <>
      <span className="label" id="set-perm">
        Default permissions
      </span>
      <div role="radiogroup" aria-labelledby="set-perm" className="perm-cards">
        {PERMISSIONS.map((p) => (
          <button
            key={p.key}
            role="radio"
            aria-checked={current === p.key}
            className={`perm-card${current === p.key ? " on" : ""}`}
            onClick={() => {
              setDefaultPermission(p.key);
              setCurrent(p.key);
            }}
          >
            <span className="perm-label">{p.label}</span>
            <span className="muted small">{p.desc}</span>
          </button>
        ))}
      </div>
      <p className="muted small">
        {current === "full_auto"
          ? "New sessions will start without permission prompts: the agent edits files and runs commands on its own. You can still pick another level for each session."
          : "New sessions start with this. You can still pick another level for each session."}
      </p>
    </>
  );
}
