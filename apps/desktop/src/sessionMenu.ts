// Right-click menu items for sessions and folders, shared by the sidebar and mission control.

import { api, type Folder, type Session } from "./api";
import { AGENTS } from "./agents";
import type { MenuItem } from "./ContextMenu";
import type { useStore } from "./store";
import { shortcutLabel } from "./shortcuts";

type Store = ReturnType<typeof useStore>;

export function sessionMenu(store: Store, s: Session): MenuItem[] {
  const info = store.agents?.find((a) => a.agent === s.agent);
  const name = AGENTS[s.agent].name;
  // A terminal reopens as a fresh shell.
  const canResume = !s.live && (s.agent === "shell" || (!!s.agentSessionId && !!info?.canResume));
  const canFork = !!s.agentSessionId && !!info?.canFork;
  const items: MenuItem[] = [
    { label: "Open", onSelect: () => store.select(s.id, { view: "focus", tab: "terminal" }) },
    {
      label: "Open in split",
      shortcut: shortcutLabel("split") || undefined,
      disabled: !store.selectedId || store.selectedId === s.id,
      hint: store.selectedId === s.id ? "This is the session already open" : undefined,
      onSelect: () => {
        store.setView("focus");
        store.setSplit(s.id);
      },
    },
    { label: "Rename…", onSelect: () => store.setRenameFor(s.id) },
    {
      label: s.onHold ? "Take off hold" : "Put on hold",
      hint: s.onHold ? "Show in active session tabs again" : "Move to On hold; the process keeps running",
      onSelect: () => void store.setOnHold(s.id, !s.onHold),
    },
    "separator",
  ];
  if (!s.live) {
    items.push({
      label: s.agent === "shell" ? "Reopen terminal" : "Resume here",
      disabled: !canResume || store.restarting.has(s.id),
      hint: canResume ? undefined : `${name}'s session id isn't known, or ${name} can't resume`,
      onSelect: () =>
        void store.run(async () => {
          const r = await api.restart(s.id);
          store.show(r);
        }),
    });
  }
  items.push(
    {
      label: "Fork session",
      disabled: !canFork,
      hint: canFork ? undefined : `${name} can't fork this session`,
      onSelect: () =>
        void store.run(async () => {
          const f = await api.fork(s.id);
          store.show(f);
        }),
    },
    { label: "Hand off to…", shortcut: shortcutLabel("handOff") || undefined, onSelect: () => store.setHandOffFor(s.id) },
    {
      label: "Open in terminal",
      disabled: s.live || !s.agentSessionId,
      hint: s.live ? "Stop it here first: two terminals on one conversation would collide" : undefined,
      onSelect: () =>
        void store.run(async () => {
          const d = await api.details(s.id);
          if (!d.resumeCommand) throw `${name}'s session id isn't known`;
          await api.openInTerminal(d.cwd, d.resumeCommand);
        }),
    },
    "separator",
  );
  if (s.live) {
    const restarting = store.restarting.has(s.id);
    const canRestart = !!s.agentSessionId && !!info?.canResume;
    items.push({
      label: restarting ? "Restarting…" : s.updateReady ? "Restart to update" : "Restart session",
      disabled: restarting || !canRestart,
      hint: !canRestart
        ? "The conversation must be resumable before restarting"
        : s.origin === "attached"
          ? `${name} restarts its background session on this conversation; interrupts any work in progress`
          : "Stops the current process and resumes this conversation; interrupts any work in progress",
      onSelect: () => void store.restartSession(s.id),
    });
    items.push({ label: "Stop session", danger: true, disabled: store.restarting.has(s.id), onSelect: () => void store.run(() => api.stop(s.id)) });
  }
  items.push({
    label: s.live ? "Stop and remove from Termlab" : "Remove from Termlab",
    danger: true,
    disabled: store.restarting.has(s.id),
    hint: `Removes it from Termlab; ${name}'s own history is kept`,
    onSelect: () => void store.run(() => api.deleteSession(s.id)),
  });
  return items;
}

export function folderMenu(store: Store, f: Folder): MenuItem[] {
  const inFolder = store.sessions.filter((s) => s.folderId === f.id);
  const live = inFolder.filter((s) => s.live);
  return [
    { label: "New session here", onSelect: () => store.openNewSession({ folderId: f.id }) },
    { label: "Edit .env…", onSelect: () => store.setEnvFolder(f) },
    { label: "Sessions in this folder…", onSelect: () => store.openNewSession({ tab: "resume", folderId: f.id }) },
    {
      label: "Broadcast to running sessions…",
      disabled: live.length === 0,
      hint: live.length === 0 ? "No sessions are running here" : undefined,
      onSelect: () => store.setBroadcastFor(f.id),
    },
    {
      label: "Stop idle sessions",
      disabled: !live.some((s) => s.status === "idle"),
      onSelect: () =>
        void store.run(async () => {
          const n = await api.stopIdle(f.id);
          store.toast(`Stopped ${n} idle session${n === 1 ? "" : "s"} in ${f.name}`, "info");
        }),
    },
    "separator",
    {
      label: "Remove folder",
      danger: true,
      disabled: live.length > 0,
      hint: live.length > 0 ? "Stop its running sessions first" : undefined,
      onSelect: () =>
        store.confirm({
          title: `Remove ${f.name}?`,
          body: `This removes the folder and its ${inFolder.length} session${inFolder.length === 1 ? "" : "s"} from Termlab, including their saved terminal output. The folder itself and the agents' own histories are not touched.`,
          action: "Remove folder",
          onConfirm: () => void store.run(() => api.removeFolder(f.id)),
        }),
    },
  ];
}
