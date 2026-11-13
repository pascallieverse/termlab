import { useEffect, useState } from "react";
import { api, errorText, type AgentId, type PermissionPreset } from "./api";
import { AGENTS, PERMISSIONS, defaultPermission } from "./agents";
import { Dialog } from "./Dialog";
import { useStore } from "./store";
import { Icon, Monogram } from "./ui";

/** Start a different agent in the same folder, briefed from the transcript and diff. */
export function HandOffDialog() {
  const store = useStore();
  const source = store.sessions.find((s) => s.id === store.handOffFor);
  const installed = (store.agents ?? []).filter((a) => a.installed);
  const [agent, setAgent] = useState<AgentId | null>(
    installed.find((a) => a.agent !== source?.agent)?.agent ?? installed[0]?.agent ?? null,
  );
  const [permission, setPermission] = useState<PermissionPreset>(source?.permissionMode ?? defaultPermission());
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const close = () => store.setHandOffFor(null);

  useEffect(() => {
    if (!source) return;
    api.handoffMessage(source.id).then(setMessage, (e) => setMessage(`(couldn't build the brief: ${errorText(e)})`));
  }, [source]);

  if (!source) return null;

  async function start() {
    if (!agent || !source) return;
    setBusy(true);
    const s = await store.run(() => api.handOff(source.id, agent, permission, null));
    setBusy(false);
    if (s) {
      close();
      store.select(s.id, { view: "focus", tab: "terminal" });
    }
  }

  return (
    <Dialog onClose={close} width={640} labelledBy="ho-title">
      <div className="dialog-head">
        <h1 id="ho-title">Hand off to another agent</h1>
        <div className="spacer" />
        <button className="icon-btn" aria-label="Close" onClick={close}>
          <Icon name="close" size={16} />
        </button>
      </div>
      <div className="dialog-body">
        <p className="muted small">
          Starts the chosen agent in the same folder with a first message built from “{source.title}”: its
          goal, where it got to, and the files it changed. {AGENTS[source.agent].name} keeps running.
        </p>
        <fieldset className="field">
          <legend className="label">Agent</legend>
          <div className="agent-cards">
            {(store.agents ?? []).map((a) => (
              <button
                key={a.agent}
                className={`agent-card${agent === a.agent ? " on" : ""}`}
                aria-pressed={agent === a.agent}
                disabled={!a.installed}
                onClick={() => setAgent(a.agent)}
              >
                <Monogram agent={a.agent} size={32} />
                <span className="agent-card-name">{a.name}</span>
                {!a.installed && <span className="faint small">Not installed</span>}
              </button>
            ))}
          </div>
        </fieldset>
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
        <div className="field">
          <span className="label">First message</span>
          <pre className="brief mono">{message ?? "Building the brief…"}</pre>
        </div>
      </div>
      <div className="dialog-foot">
        <div className="spacer" />
        <button className="btn tall" onClick={close}>
          Cancel
        </button>
        <button className="btn primary tall" disabled={!agent || busy} onClick={() => void start()}>
          Hand off
        </button>
      </div>
    </Dialog>
  );
}

/** Sends one message to every running session in a folder. */
export function BroadcastDialog() {
  const store = useStore();
  const folder = store.folders.find((f) => f.id === store.broadcastFor);
  const [text, setText] = useState("");
  const close = () => store.setBroadcastFor(null);
  if (!folder) return null;
  const running = store.sessions.filter((s) => s.folderId === folder.id && s.live).length;

  async function send() {
    if (!folder || !text.trim()) return;
    const n = await store.run(() => api.broadcast(folder.id, text));
    if (n !== undefined) {
      store.toast(`Sent to ${n} session${n === 1 ? "" : "s"} in ${folder.name}`, "info");
      close();
    }
  }

  return (
    <Dialog onClose={close} width={560} labelledBy="bc-title">
      <div className="dialog-head">
        <h1 id="bc-title">Broadcast to {folder.name}</h1>
        <div className="spacer" />
        <button className="icon-btn" aria-label="Close" onClick={close}>
          <Icon name="close" size={16} />
        </button>
      </div>
      <div className="dialog-body">
        <p className="muted small">
          Types this message into all {running} running session{running === 1 ? "" : "s"} in {folder.name} and
          presses Enter.
        </p>
        <label htmlFor="bc-text" className="sr">
          Message
        </label>
        <textarea
          id="bc-text"
          data-autofocus
          className="textarea"
          rows={4}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              void send();
            }
          }}
        />
      </div>
      <div className="dialog-foot">
        <div className="spacer" />
        <button className="btn tall" onClick={close}>
          Cancel
        </button>
        <button className="btn primary tall" disabled={!text.trim() || running === 0} onClick={() => void send()}>
          Send to {running} <span className="mono kbd-hint">⌘↵</span>
        </button>
      </div>
    </Dialog>
  );
}
