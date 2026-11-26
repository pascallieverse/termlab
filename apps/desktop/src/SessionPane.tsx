import { api, type ApprovalChoice, type Session } from "./api";
import { TruncatedText } from "./TruncatedText";
import { AGENTS, sessionColor, tildify } from "./agents";
import { DiffView } from "./DiffView";
import { TerminalView } from "./TerminalView";
import { TimelineView } from "./TimelineView";
import { useStore, type InspectorTab, type Tab } from "./store";
import { Icon, Monogram, StatusPill } from "./ui";
import { useDetails } from "./useDetails";
import { shortcutHint } from "./shortcuts";
import { beginSessionDrag } from "./sessionDrag";
import { LabelsButton, SessionLabels } from "./Labels";

/** Buttons and tabs in a pane's header keep their own clicks; the rest of it drags the pane. */
const onControl = (target: EventTarget) => target instanceof Element && target.closest("button, [role='tablist']") !== null;

/**
 * One session: header with tabs, terminal (or diff/timeline), approval bar, composer.
 * With several panes open (`multi`) the header is a single compact line that also drags the
 * pane, and only the active pane shows the tabs and the inspector buttons.
 */
export function SessionPane({ session: s, active = true, multi = false }: { session: Session; active?: boolean; multi?: boolean }) {
  const store = useStore();
  const folder = store.folderOf(s);
  const details = useDetails(s.id, s.status);
  const agent = AGENTS[s.agent];
  // From the agent itself, not the session's details: those load a moment after switching.
  const canResume = store.agents?.find((a) => a.agent === s.agent)?.canResume ?? false;
  const tab = store.tabOf(s.id);
  const tint = sessionColor(s.color);

  return (
    <section
      className={`session-pane${multi ? " in-split" : ""}${multi && active ? " is-active" : ""}`}
      aria-label={s.title}
      aria-current={multi && active ? "true" : undefined}
      // Pressing anywhere in a pane makes it the active one.
      onPointerDownCapture={multi && !active ? () => store.focusPane(s.id) : undefined}
    >
      <div
        className={`session-head${multi ? " compact" : ""}${tint.className}`}
        style={tint.style}
        onPointerDown={multi ? (e) => !onControl(e.target) && beginSessionDrag(e, s) : undefined}
        // A press on the header leaves the keyboard in the pane's terminal.
        onMouseDown={multi ? (e) => !onControl(e.target) && e.preventDefault() : undefined}
      >
        <Monogram agent={s.agent} size={multi ? 26 : 38} />
        <div className="session-title">
          <TruncatedText as="h1" text={s.title} />
          {!multi && (
            <div className="session-meta">
              <span>{agent.name}</span>
              <span aria-hidden="true">·</span>
              <span className="mono">{s.model ?? details?.usage?.model ?? "default model"}</span>
              {details?.branch && (
                <>
                  <span aria-hidden="true">·</span>
                  <span className="mono">⎇ {details.branch}</span>
                </>
              )}
              <LabelsButton sessionId={s.id} />
            </div>
          )}
        </div>
        {multi && <SessionLabels sessionId={s.id} dots />}
        <StatusPill session={s} compact={multi} />
        <div className="spacer" />
        {active && (
          <>
            {s.live && (s.updateReady || !!s.updateAvailable || store.restarting.has(s.id)) && !!s.agentSessionId && canResume && (
              <UpdateButton session={s} />
            )}
            <Tabs current={tab} onChange={(t) => store.setTabFor(s.id, t)} />
            <InspectorButton tab="notes" label="project notes" icon="note" />
            <InspectorButton tab="session" label="session details" icon="panel" />
          </>
        )}
        {multi && (
          <button
            className="icon-btn"
            aria-label={`Close pane: ${s.title}`}
            title={`Close pane${active ? shortcutHint("closePane") : ""}`}
            onClick={() => store.closePane(s.id)}
          >
            <Icon name="close" />
          </button>
        )}
      </div>

      {s.onHold && (
        <div role="status" className="banner muted">
          <span>On hold{s.live ? " · The process is still running." : ""}</span>
          <div className="spacer" />
          <button className="btn" onClick={() => void store.setOnHold(s.id, false)}>Take off hold</button>
        </div>
      )}
      {s.apiKeyVars.length > 0 && (
        <div role="status" className="banner warn">
          <Icon name="warning" /> This session is billed to an API key ({s.apiKeyVars.join(", ")} is set in
          your environment).
        </div>
      )}
      {!s.live && !store.restarting.has(s.id) && <EndedBar session={s} canResume={details?.canResume ?? false} />}

      {store.restarting.has(s.id) && <div className="banner muted" role="status">Restarting session…</div>}
      <div className="session-body">
        {tab === "terminal" &&
          (s.origin === "imported" && !s.live && s.exitCode === null ? (
            <ImportedPanel session={s} canResume={details?.canResume ?? false} />
          ) : (
            <TerminalView
              key={`${s.id}:${s.live}:${store.terminalEpochs[s.id] ?? 0}`}
              sessionId={s.id}
              agent={s.agent}
              live={s.live}
              autoFocus={active}
            />
          ))}
        {tab === "diff" && <DiffView sessionId={s.id} status={s.status} />}
        {tab === "timeline" && <TimelineView sessionId={s.id} status={s.status} />}
      </div>

      {s.live && s.status === "needs_you" && <ApprovalBar session={s} />}
      {!s.live && folder && (
        <div className="composer-hint faint mono">{tildify(folder.path, store.home)}</div>
      )}
    </section>
  );
}

/** Opens the right column on one of its tabs, or closes it when that tab is showing. */
function InspectorButton({ tab, label, icon }: { tab: InspectorTab; label: string; icon: "note" | "panel" }) {
  const store = useStore();
  const on = store.inspectorOpen && store.inspectorTab === tab;
  return (
    <button
      className={`icon-btn head-extra${on ? " on" : ""}`}
      aria-label={label[0].toUpperCase() + label.slice(1)}
      aria-pressed={on}
      title={`${on ? "Hide" : "Show"} ${label}${tab === "session" ? shortcutHint("inspector") : ""}`}
      onClick={() => store.showInspector(tab)}
    >
      <Icon name={icon} size={16} />
    </button>
  );
}

/**
 * A newer version of the agent was installed while this session kept running the older one,
 * or (for agents that don't update themselves, like Codex) a newer release is out. Restarting
 * the session (on the same conversation) is what puts it on the new version; in the second
 * case Termlab first installs it into the copy this session runs.
 */
function UpdateButton({ session: s }: { session: Session }) {
  const store = useStore();
  const name = AGENTS[s.agent].name;
  const restarting = store.restarting.has(s.id);
  const busy = s.status === "running" || s.status === "needs_you";
  const install = !s.updateReady && !!s.updateAvailable;
  const go = () => void store.restartSession(s.id, install);

  function restart() {
    if (!busy) return go();
    store.confirm({
      title: install ? `Update ${name} now?` : `Restart ${name} now?`,
      body:
        s.status === "running"
          ? `${name} is working. Restarting interrupts that; the conversation continues on the new version.`
          : `${name} is waiting for your answer. Restarting drops that question; the conversation continues on the new version.`,
      action: install ? "Update and restart" : "Restart",
      onConfirm: go,
    });
  }

  return (
    <button
      className="btn small amber-outline update-btn"
      disabled={restarting}
      title={
        install
          ? `${name} ${s.updateAvailable} is out. Termlab installs it into the copy this session runs, then restarts the session on the same conversation.`
          : `A newer ${name} is installed; this session still runs the older one. Restarting continues the same conversation on the new version.`
      }
      onClick={restart}
    >
      <Icon name="refresh" /> {restarting ? (install ? "Updating…" : "Restarting…") : install ? `Update to ${s.updateAvailable}` : "Restart to update"}
    </button>
  );
}

/** An imported session has no Termlab output yet: its history is in the agent's own files. */
function ImportedPanel({ session: s, canResume }: { session: Session; canResume: boolean }) {
  const store = useStore();
  return (
    <div className="imported-panel">
      <Monogram agent={s.agent} size={38} />
      <h2>Imported from {AGENTS[s.agent].name}</h2>
      <p className="muted">
        This conversation was loaded from {AGENTS[s.agent].name}'s own history, so Termlab has no terminal
        output for it yet. Its steps are in the Timeline tab.
      </p>
      <div className="imported-actions">
        {canResume && (
          <button className="btn primary tall" onClick={() => void store.run(() => api.restart(s.id))}>
            <Icon name="refresh" /> Resume here
          </button>
        )}
        <button className="btn tall" onClick={() => store.setTabFor(s.id, "timeline")}>
          Open timeline
        </button>
      </div>
    </div>
  );
}

function Tabs({ current, onChange }: { current: Tab; onChange(t: Tab): void }) {
  const tabs: [Tab, string][] = [
    ["terminal", "Terminal"],
    ["diff", "Diff"],
    ["timeline", "Timeline"],
  ];
  return (
    <div role="tablist" aria-label="Session view" className="segmented">
      {tabs.map(([key, label]) => (
        <button
          key={key}
          role="tab"
          aria-selected={current === key}
          className={current === key ? "on" : ""}
          onClick={() => onChange(key)}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

/** Why "Resume here" isn't offered for an ended session. */
function whyNotResumable(s: Session): string {
  const name = AGENTS[s.agent].name;
  if (s.agentSessionId) return `${name} can't resume sessions from Termlab.`;
  if (s.agent === "vibe") return "Mistral Vibe saves a conversation only once it has a message, so there's nothing to resume.";
  return `${name}'s session id was never found, so it can't be resumed.`;
}

function EndedBar({ session: s, canResume }: { session: Session; canResume: boolean }) {
  const store = useStore();
  // No exit code: Termlab's background service stopped (or lost) the session.
  const text =
    s.origin === "imported" && s.exitCode === null
      ? `Imported from ${AGENTS[s.agent].name}'s history.`
      : s.exitCode === null || s.exitCode === -1
        ? "This session stopped when Termlab's background service did."
        : s.status === "done"
          ? "This session has ended."
          : `This session exited with code ${s.exitCode}.`;
  return (
    <div className="banner muted">
      <span>{text}</span>
      <div className="spacer" />
      {!canResume && <span className="muted small">{whyNotResumable(s)}</span>}
      {canResume && (
        <button className="btn" onClick={() => void store.run(() => api.restart(s.id))}>
          <Icon name="refresh" /> Resume here
        </button>
      )}
      <button
        className="btn ghost-danger"
        onClick={() => void store.run(() => api.deleteSession(s.id))}
        title="Remove this session and its saved output from Termlab"
      >
        Remove
      </button>
    </div>
  );
}

/** Answers the agent's permission prompt by sending the matching keys to its pty. */
function ApprovalBar({ session: s }: { session: Session }) {
  const store = useStore();
  const a = s.approval;
  const has = (c: ApprovalChoice) => c === "deny" || (a?.options.some((o) => o.choice === c) ?? false);
  const answer = (c: ApprovalChoice) => void store.run(() => api.answer(s.id, c));
  const parsed = (a?.options.length ?? 0) > 0;
  const question = a?.kind === "question";
  return (
    <div className="approval" role="region" aria-label={question ? "Question" : "Permission request"}>
      <div className="approval-text">
        <span className="approval-title">
          {AGENTS[s.agent].name} {question ? "has a question for you" : "is asking for permission"}
        </span>
        {a?.detail && <code className="mono">{a.detail}</code>}
        {a?.question && <span className="approval-question">{a.question}</span>}
        {!parsed && <span className="approval-question">Answer in the terminal above.</span>}
      </div>
      {parsed && (
        <div className="approval-actions">
          {has("allow_once") && (
            <button className="btn primary" onClick={() => answer("allow_once")}>
              Allow once
            </button>
          )}
          {has("allow_always") && (
            <button className="btn amber-outline" onClick={() => answer("allow_always")}>
              Always allow in session
            </button>
          )}
          <button className="btn outline" onClick={() => answer("deny")}>
            Deny
          </button>
        </div>
      )}
    </div>
  );
}
