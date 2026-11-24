import { EnvDialog } from "./EnvDialog";
import { UsageChip } from "./UsageChip";
import { useEffect } from "react";
import { formatCost } from "./agents";
import { api } from "./api";
import { BroadcastDialog, HandOffDialog } from "./HandOffDialog";
import { Inspector } from "./Inspector";
import { MissionControl } from "./MissionControl";
import { NewSessionDialog } from "./NewSessionDialog";
import { Palette } from "./Palette";
import { SessionPane } from "./SessionPane";
import { Sidebar } from "./Sidebar";
import { ConfirmDialog, RenameDialog, SubagentDialog } from "./SmallDialogs";
import { LabelsDialog } from "./Labels";
import { SettingsDialog } from "./SettingsDialog";
import { UpdateBanner, startUpdateChecks } from "./updates";
import { StoreProvider, useStore } from "./store";
import { TerminalView } from "./TerminalView";
import { Icon, Kbd, Logo, Monogram, StatusPill } from "./ui";
import { shortcutForEvent } from "./keybindings";
import { IS_MAC, getBindings, isRecordingShortcut, shortcutLabel, useBindings } from "./shortcuts";

export function App() {
  return (
    <StoreProvider>
      <Shell />
    </StoreProvider>
  );
}

/** Restarts an outdated background service; it asks first when agents are mid-turn. */
function RestartServiceButton() {
  const store = useStore();
  const busy = store.sessions.filter((s) => s.live && s.status === "running");
  const restart = () => void store.run(() => api.restartDaemon());
  return (
    <button
      className="btn small"
      onClick={() =>
        busy.length === 0
          ? restart()
          : store.confirm({
              title: "Restart the background service?",
              body: `${busy.length} session${busy.length === 1 ? " is" : "s are"} working right now (${busy
                .map((s) => s.title)
                .slice(0, 3)
                .join(", ")}${busy.length > 3 ? ", …" : ""}). Every session is reopened with its conversation after the restart, but work in progress, including running sub-agents, stops.`,
              action: "Restart now",
              onConfirm: restart,
            })
      }
    >
      Restart service
    </button>
  );
}

function Shell() {
  const store = useStore();
  const selected = store.visibleSessions.find((s) => s.id === store.selectedId) ?? null;
  const split = store.visibleSessions.find((s) => s.id === store.splitId) ?? null;
  useShortcuts();
  useEffect(startUpdateChecks, []);

  return (
    <div className="app">
      <Header />
      <UpdateBanner />
      {store.daemon.message && (
        <div role="status" className="banner daemon">
          <span>{store.daemon.message}</span>
          {store.daemon.outdated && <RestartServiceButton />}
        </div>
      )}
      <div className="workspace">
        <Sidebar />
        {store.view === "mission" ? (
          <main className="main">
            <MissionControl />
          </main>
        ) : (
          <>
            <main className="main">
              {selected ? (
                <div className={`panes${split ? " split" : ""}`}>
                  <SessionPane session={selected} />
                  {split && <SplitPane id={split.id} />}
                </div>
              ) : (
                <Empty />
              )}
            </main>
            {selected && store.inspectorOpen && <Inspector session={selected} />}
          </>
        )}
      </div>

      {store.newSession && <NewSessionDialog />}
      {store.paletteOpen && <Palette />}
      {store.handOffFor && <HandOffDialog />}
      {store.broadcastFor !== null && <BroadcastDialog />}
      {store.envFolder && <EnvDialog key={store.envFolder.id} folder={store.envFolder} />}
      {store.renameFor && <RenameDialog />}
      {store.labelsFor && <LabelsDialog />}
      {store.subagentView && <SubagentDialog />}
      {store.settingsOpen && <SettingsDialog />}
      {store.confirmRequest && <ConfirmDialog />}
      <Toasts />
    </div>
  );
}

function Header() {
  const store = useStore();
  const bindings = useBindings();
  const running = store.sessions.filter((s) => !s.onHold && s.status === "running").length;
  const needs = store.sessions.filter((s) => !s.onHold && s.status === "needs_you").length;
  const installed = (store.agents ?? []).filter((a) => a.installed && a.agent !== "shell").length;
  const startOfDay = new Date().setHours(0, 0, 0, 0);
  // Spend is shown only for API-key sessions; subscription sessions have no per-session cost.
  const spend = store.sessions
    .filter((s) => s.billing === "api_key" && s.lastActivityAt >= startOfDay)
    .reduce((sum, s) => sum + (s.usage?.costUsd ?? 0), 0);
  const anyApiKey = store.sessions.some((s) => s.billing === "api_key");

  return (
    <header className="app-header">
      <div className="brand">
        <Logo />
        <span>Termlab</span>
      </div>
      <nav aria-label="Views" className="segmented">
        <button className={store.view === "focus" ? "on" : ""} aria-pressed={store.view === "focus"} onClick={() => store.setView("focus")}>
          Focus
        </button>
        <button className={store.view === "mission" ? "on" : ""} aria-pressed={store.view === "mission"} onClick={() => store.setView("mission")}>
          Mission control
        </button>
      </nav>
      <div className="header-chips">
        <span className="hchip">
          <span style={{ color: "var(--running)" }}>●</span> {running} running
        </span>
        {needs > 0 && <span className="hchip amber">▲ {needs} need you</span>}
        <span className="hchip">
          {anyApiKey ? `${formatCost(spend)} today · ` : ""}
          {installed} agent{installed === 1 ? "" : "s"}
        </span>
        <UsageChip />
      </div>
      <div className="spacer" />
      <button className="btn search-btn" onClick={() => store.setPaletteOpen(true)}>
        <Icon name="search" /> <span className="search-label">Search or run a command</span>{" "}
        {shortcutLabel("palette", bindings) && <Kbd>{shortcutLabel("palette", bindings)}</Kbd>}
      </button>
      <button className="btn primary" onClick={() => store.openNewSession()}>
        <Icon name="plus" /> New session
      </button>
      <button className="icon-btn" aria-label="Settings" title="Settings" onClick={() => store.setSettingsOpen(true)}>
        <Icon name="settings" size={16} />
      </button>
    </header>
  );
}

function SplitPane({ id }: { id: string }) {
  const store = useStore();
  const s = store.sessions.find((x) => x.id === id);
  if (!s) return null;
  return (
    <section className="split-pane" aria-label={`Split: ${s.title}`}>
      <div className="split-head">
        <Monogram agent={s.agent} size={26} />
        <button className="link-btn split-title" onClick={() => store.select(s.id)} title="Make this the main session">
          {s.title}
        </button>
        <StatusPill session={s} compact />
        <div className="spacer" />
        <button className="icon-btn" aria-label="Close split" onClick={() => store.setSplit(null)}>
          <Icon name="close" />
        </button>
      </div>
      <TerminalView
        key={`${s.id}:${s.live}:${store.terminalEpochs[s.id] ?? 0}`}
        sessionId={s.id}
        agent={s.agent}
        live={s.live}
        compact
        autoFocus={false}
      />
    </section>
  );
}

function Empty() {
  const store = useStore();
  const bindings = useBindings();
  if (store.sessionFilter === "on_hold") {
    const hasHeldSessions = store.visibleSessions.length > 0;
    return (
      <div className="empty">
        <Logo />
        <h2>{hasHeldSessions ? "No session selected" : "No sessions on hold"}</h2>
        <p className="muted">
          {hasHeldSessions
            ? "Pick an on-hold session on the left."
            : "Sessions you put on hold will appear here."}
        </p>
      </div>
    );
  }
  return (
    <div className="empty">
      <Logo />
      {store.folders.length === 0 ? (
        <>
          <h2>Add a project folder to get started</h2>
          <p className="muted">Termlab runs your agents' own CLIs in each folder, using their own logins.</p>
        </>
      ) : (
        <>
          <h2>No session selected</h2>
          <p className="muted">Pick one on the left, or start a new one.</p>
        </>
      )}
      <div className="empty-actions">
        <button className="btn primary tall" onClick={() => store.openNewSession()}>
          <Icon name="plus" /> New session <span className="mono kbd-hint">{shortcutLabel("newSession", bindings)}</span>
        </button>
        <button className="btn tall" onClick={() => store.openNewSession({ tab: "resume", everywhere: true })}>
          <Icon name="refresh" /> Load sessions from your agents…
        </button>
      </div>
    </div>
  );
}

function Toasts() {
  const store = useStore();
  return (
    <div className="toasts" role="status" aria-live="polite">
      {store.toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          <span>{t.text}</span>
          <button className="icon-btn" aria-label="Dismiss" onClick={() => store.dismissToast(t.id)}>
            <Icon name="close" />
          </button>
        </div>
      ))}
    </div>
  );
}

function useShortcuts() {
  const store = useStore();
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.repeat || e.isComposing || isRecordingShortcut()) return;
      const id = shortcutForEvent(e, getBindings(), IS_MAC);
      if (!id) return;
      if ((id === "split" || id === "handOff") && !store.selectedId) return;
      e.preventDefault();
      switch (id) {
        case "palette":
          store.setPaletteOpen(!store.paletteOpen);
          break;
        case "newSession":
          store.openNewSession();
          break;
        case "settings":
          store.setSettingsOpen(!store.settingsOpen);
          break;
        case "focusView":
          store.setView("focus");
          break;
        case "missionView":
          store.setView("mission");
          break;
        case "inspector":
          store.toggleInspector();
          break;
        case "handOff":
          store.setHandOffFor(store.selectedId);
          break;
        case "split":
          if (store.splitId) store.setSplit(null);
          else {
            const other = store.visibleSessions.find((s) => s.id !== store.selectedId && s.live) ??
              store.visibleSessions.find((s) => s.id !== store.selectedId);
            if (other) store.setSplit(other.id);
          }
          break;
      }
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [store]);
}
