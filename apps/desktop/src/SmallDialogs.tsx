import { useEffect, useState } from "react";
import { TimelineView } from "./TimelineView";
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
