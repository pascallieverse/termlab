import { api, type ApprovalChoice, type Session } from "./api";
import { TruncatedText } from "./TruncatedText";
import { AGENTS, sessionColor, tildify } from "./agents";
import { DiffView } from "./DiffView";
import { TerminalView } from "./TerminalView";
import { TimelineView } from "./TimelineView";
import { useStore, type Tab } from "./store";
import { Icon, Monogram, StatusPill } from "./ui";
import { useDetails } from "./useDetails";

/** The selected session: header with tabs, terminal (or diff/timeline), approval bar, composer. */
export function SessionPane({ session: s }: { session: Session }) {
  const store = useStore();
  const folder = store.folderOf(s);
  const details = useDetails(s.id, s.status);
  const agent = AGENTS[s.agent];

  return (
    <section className="session-pane" aria-label={s.title}>
      <div className={`session-head${sessionColor(s.color).className}`} style={sessionColor(s.color).style}>
        <Monogram agent={s.agent} size={38} />
        <div className="session-title">
          <TruncatedText as="h1" text={s.title} />
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
          </div>
        </div>
        <StatusPill session={s} />
        <div className="spacer" />
        <Tabs current={store.tab} onChange={store.setTab} />
        <button
          className={`icon-btn${store.inspectorOpen ? " on" : ""}`}
          aria-label="Session details"
          aria-pressed={store.inspectorOpen}
          title={`${store.inspectorOpen ? "Hide" : "Show"} session details (⌘I)`}
          onClick={store.toggleInspector}
        >
          <Icon name="panel" size={16} />
        </button>
      </div>

      {s.apiKeyVars.length > 0 && (
        <div role="status" className="banner warn">
          <Icon name="warning" /> This session is billed to an API key ({s.apiKeyVars.join(", ")} is set in
          your environment).
        </div>
      )}
      {!s.live && !store.restarting.has(s.id) && <EndedBar session={s} canResume={details?.canResume ?? false} />}

      {store.restarting.has(s.id) && <div className="banner muted" role="status">Restarting session…</div>}
      <div className="session-body">
        {store.tab === "terminal" &&
          (s.origin === "imported" && !s.live && s.exitCode === null ? (
            <ImportedPanel session={s} canResume={details?.canResume ?? false} />
          ) : (
            <TerminalView key={`${s.id}:${s.live}:${store.terminalEpochs[s.id] ?? 0}`} sessionId={s.id} agent={s.agent} live={s.live} />
          ))}
        {store.tab === "diff" && <DiffView sessionId={s.id} status={s.status} />}
        {store.tab === "timeline" && <TimelineView sessionId={s.id} status={s.status} />}
      </div>

      {s.live && s.status === "needs_you" && <ApprovalBar session={s} />}
      {!s.live && folder && (
        <div className="composer-hint faint mono">{tildify(folder.path, store.home)}</div>
      )}
    </section>
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
        <button className="btn tall" onClick={() => store.setTab("timeline")}>
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
  return (
    <div className="approval" role="region" aria-label="Permission request">
      <div className="approval-text">
        <span className="approval-title">{AGENTS[s.agent].name} is asking for permission</span>
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
