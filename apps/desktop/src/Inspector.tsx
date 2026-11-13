import { api, type Session } from "./api";
import { AGENTS, formatCost, formatTokens, permissionLabel, tildify } from "./agents";
import { useStore } from "./store";
import { useDetails } from "./useDetails";

export function Inspector({ session: s }: { session: Session }) {
  const store = useStore();
  const details = useDetails(s.id, s.status);
  const folder = store.folderOf(s);
  const usage = details?.usage ?? s.usage;
  const ctx = details?.contextPercent ?? s.contextPercent;
  const ctxColor = ctx === null ? "var(--running)" : ctx > 80 ? "var(--error)" : ctx > 60 ? "var(--accent)" : "var(--running)";
  const agentInfo = store.agents?.find((a) => a.agent === s.agent);
  const forkTip = !agentInfo?.canFork
    ? `${AGENTS[s.agent].name} can't fork sessions`
    : !s.agentSessionId
      ? "The agent's session id isn't known yet"
      : "Start a new session that continues from this conversation";

  return (
    <aside className="inspector" aria-label="Session details">
      <section>
        <h2 className="section-heading">Session</h2>
        <dl className="facts">
          <dt>Agent</dt>
          <dd>{AGENTS[s.agent].name}</dd>
          <dt>Model</dt>
          <dd className="mono small">{s.model ?? usage?.model ?? "default"}</dd>
          <dt>Directory</dt>
          <dd className="mono small wrap">{folder ? tildify(folder.path, store.home) : "—"}</dd>
          <dt>Branch</dt>
          <dd className="mono small">{details?.branch ?? "—"}</dd>
          <dt>Isolation</dt>
          <dd>Current checkout</dd>
          <dt>Permissions</dt>
          <dd>{permissionLabel(s.permissionMode)}</dd>
          <dt>Resume ID</dt>
          <dd className="mono small wrap" title={s.agentSessionId ?? undefined}>
            {s.agentSessionId ? shortId(s.agentSessionId) : "—"}
          </dd>
        </dl>
      </section>

      <section>
        <h2 className="section-heading">Context &amp; spend</h2>
        <div className="row-between">
          <span className="muted">Context window</span>
          <span>{ctx === null ? "—" : `${ctx}%`}</span>
        </div>
        <div
          className="meter"
          role="img"
          aria-label={ctx === null ? "Context window use unknown" : `Context window ${ctx} percent used`}
        >
          <div style={{ width: `${ctx ?? 0}%`, background: ctxColor }} />
        </div>
        <div className="stat-grid">
          <div className="stat">
            <div className="stat-label">Tokens</div>
            <div className="stat-value">{usage ? formatTokens(usage.totalTokens) : "—"}</div>
          </div>
          <div className="stat">
            <div className="stat-label">{s.billing === "api_key" ? "Cost" : "Billing"}</div>
            <div className="stat-value">
              {s.billing === "api_key"
                ? usage?.costUsd != null
                  ? formatCost(usage.costUsd)
                  : "API key"
                : s.billing === "subscription"
                  ? "Subscription"
                  : "—"}
            </div>
          </div>
        </div>
        {!usage && <p className="faint small">Token counts come from the agent's transcript, when it writes one.</p>}
      </section>

      <section>
        <h2 className="section-heading">Changed files</h2>
        {!s.startCommit && <p className="faint small">Not a git repository when the session started.</p>}
        {s.startCommit && details && details.files.length === 0 && <p className="muted small">No changes yet.</p>}
        {details?.files.slice(0, 30).map((f) => (
          <div key={f.path} className="file-row mono">
            <span className="file-path" title={f.path}>
              {f.path}
            </span>
            {f.added === null ? (
              <span className="faint">bin</span>
            ) : (
              <>
                <span className="add">+{f.added}</span>
                <span className="del">−{f.deleted}</span>
              </>
            )}
          </div>
        ))}
        {details && details.files.length > 30 && (
          <button className="link-btn" onClick={() => store.setTab("diff")}>
            {details.files.length - 30} more…
          </button>
        )}
      </section>

      <section className="actions">
        <h2 className="section-heading">Actions</h2>
        <div className="action-grid">
          <button
            className="btn"
            disabled={!details?.resumeCommand || s.live}
            title={
              s.live
                ? "Stop the session here first: two terminals on one conversation would collide"
                : details?.resumeCommand
                  ? `Runs: ${details.resumeCommand}`
                  : `${AGENTS[s.agent].name}'s session id isn't known`
            }
            onClick={() =>
              details?.resumeCommand &&
              void store.run(() => api.openInTerminal(details.cwd, details.resumeCommand!))
            }
          >
            Open in terminal
          </button>
          <button
            className="btn"
            disabled={!details?.canFork}
            title={forkTip}
            onClick={() =>
              void store.run(async () => {
                const f = await api.fork(s.id);
                store.select(f.id, { tab: "terminal" });
              })
            }
          >
            Fork session
          </button>
          <button className="btn" onClick={() => store.setHandOffFor(s.id)}>
            Hand off to…
          </button>
          <button className="btn" disabled={!s.startCommit} onClick={() => store.setTab("diff")}>
            Review diff
          </button>
        </div>
        {s.live ? (
          <button className="btn danger" onClick={() => void store.run(() => api.stop(s.id))}>
            Stop session
          </button>
        ) : (
          details?.canResume && (
            <button className="btn" onClick={() => void store.run(() => api.restart(s.id))}>
              Resume here
            </button>
          )
        )}
      </section>
    </aside>
  );
}

function shortId(id: string): string {
  return id.length > 14 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id;
}
