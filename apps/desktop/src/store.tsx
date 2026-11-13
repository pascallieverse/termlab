import { restartSession } from "./restartSession";
// App state: folders, sessions and agents from the daemon, plus UI selection and dialogs.
// Also sends native notifications and keeps the dock badge in sync.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import {
  api,
  errorText,
  onCoreEvent,
  onDaemonStatus,
  onResync,
  type AgentId,
  type AgentInfo,
  type DaemonStatus,
  type Folder,
  type Session,
} from "./api";
import { AGENTS, STATUS, homeFrom } from "./agents";

export type View = "focus" | "mission";
export type Tab = "terminal" | "diff" | "timeline";

export interface NewSessionOptions {
  tab?: "start" | "resume";
  /** Resume tab: look beyond the project folders. */
  everywhere?: boolean;
  folderId?: number;
  agent?: AgentId;
}

export interface ConfirmRequest {
  title: string;
  body: string;
  action: string;
  onConfirm(): void;
}

export interface Toast {
  id: number;
  text: string;
  kind: "error" | "info";
}

interface Store {
  folders: Folder[];
  sessions: Session[];
  agents: AgentInfo[] | null;
  daemon: DaemonStatus;
  home: string | null;
  selectedId: string | null;
  splitId: string | null;
  view: View;
  tab: Tab;
  newSession: NewSessionOptions | null;
  paletteOpen: boolean;
  handOffFor: string | null;
  broadcastFor: number | null;
  envFolder: Folder | null;
  setEnvFolder(folder: Folder | null): void;
  renameFor: string | null;
  /** The sub-agent whose steps are open. */
  subagentView: { sessionId: string; subagentId: string } | null;
  settingsOpen: boolean;
  confirmRequest: ConfirmRequest | null;
  /** The session inspector (right column); closed by default, remembered per viewer. */
  inspectorOpen: boolean;
  toasts: Toast[];

  select(id: string | null, opts?: { view?: View; tab?: Tab }): void;
  setSplit(id: string | null): void;
  setView(v: View): void;
  setTab(t: Tab): void;
  openNewSession(o?: NewSessionOptions): void;
  closeNewSession(): void;
  setPaletteOpen(open: boolean): void;
  setHandOffFor(id: string | null): void;
  setBroadcastFor(id: number | null): void;
  setRenameFor(id: string | null): void;
  setSubagentView(view: { sessionId: string; subagentId: string } | null): void;
  setSettingsOpen(open: boolean): void;
  /** Asks before a destructive action. */
  confirm(req: ConfirmRequest | null): void;
  toggleInspector(): void;
  toast(text: string, kind?: Toast["kind"]): void;
  dismissToast(id: number): void;
  /** Runs an action and shows its error as a toast. */
  run<T>(fn: () => Promise<T>): Promise<T | undefined>;
  moveFolder(id: number, before: number | null): Promise<void>;
  restarting: Set<string>;
  terminalEpochs: Record<string, number>;
  restartSession(id: string): Promise<void>;
  addFolder(path: string): Promise<Folder | undefined>;
  refreshAgents(): Promise<void>;
  folderOf(s: Session): Folder | undefined;
}

const INSPECTOR_KEY = "termlab.inspector";

const Ctx = createContext<Store | null>(null);

export function useStore(): Store {
  const s = useContext(Ctx);
  if (!s) throw new Error("useStore outside StoreProvider");
  return s;
}

export function StoreProvider({ children }: { children: ReactNode }) {
  const [folders, setFolders] = useState<Folder[]>([]);
  const restartPending = useRef(new Set<string>());
  const [restarting, setRestarting] = useState(new Set<string>());
  const [terminalEpochs, setTerminalEpochs] = useState<Record<string, number>>({});
  const [sessions, setSessions] = useState<Session[]>([]);
  const [agents, setAgents] = useState<AgentInfo[] | null>(null);
  const [daemon, setDaemon] = useState<DaemonStatus>({ connected: false, message: null });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [splitId, setSplitId] = useState<string | null>(null);
  const [view, setView] = useState<View>("focus");
  const [tab, setTab] = useState<Tab>("terminal");
  const [newSession, setNewSession] = useState<NewSessionOptions | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [handOffFor, setHandOffFor] = useState<string | null>(null);
  const [broadcastFor, setBroadcastFor] = useState<number | null>(null);
  const [envFolder, setEnvFolder] = useState<Folder | null>(null);
  const [renameFor, setRenameFor] = useState<string | null>(null);
  const [subagentView, setSubagentView] = useState<{ sessionId: string; subagentId: string } | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState<boolean>(() => {
    try {
      return localStorage.getItem(INSPECTOR_KEY) === "open";
    } catch {
      return false;
    }
  });
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastId = useRef(0);
  const prevStatus = useRef(new Map<string, Session["status"]>());
  const notifiedFocus = useRef<string | null>(null);
  const focused = useRef(true);

  const toast = useCallback((text: string, kind: Toast["kind"] = "error") => {
    const id = ++toastId.current;
    setToasts((t) => [...t.slice(-3), { id, text, kind }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "error" ? 8000 : 4000);
  }, []);

  const run = useCallback(
    async <T,>(fn: () => Promise<T>) => {
      try {
        return await fn();
      } catch (e) {
        toast(errorText(e));
        return undefined;
      }
    },
    [toast],
  );

  const reload = useCallback(async () => {
    const [f, s] = await Promise.all([api.listFolders(), api.listSessions()]);
    setFolders(f);
    setSessions(sortSessions(s));
    for (const x of s) prevStatus.current.set(x.id, x.status);
  }, []);

  const refreshAgents = useCallback(async () => {
    const a = await run(() => api.refreshAgents());
    if (a) setAgents(a);
  }, [run]);

  // Notifications: Needs you (unless you're looking at it), and finishes while unfocused.
  const maybeNotify = useCallback(async (s: Session, prev: Session["status"] | undefined) => {
    if (!s.notify || prev === s.status || prev === undefined) return;
    const needs = s.status === "needs_you";
    const finished = (s.status === "done" || s.status === "error") && prev !== "done" && prev !== "error";
    if (!needs && !finished) return;
    if (finished && focused.current) return;
    if (needs && focused.current && document.visibilityState === "visible" && selectedRef.current === s.id) return;
    let ok = await isPermissionGranted();
    if (!ok) ok = (await requestPermission()) === "granted";
    if (!ok) return;
    const agent = AGENTS[s.agent].name;
    sendNotification({
      title: needs ? `${agent} needs you` : `${agent} ${STATUS[s.status].label.toLowerCase()}`,
      body: needs ? (s.approval?.detail ?? s.approval?.question ?? s.title) : s.title,
    });
    // Clicking a notification brings the app forward; focus goes to this session.
    notifiedFocus.current = s.id;
  }, []);

  const selectedRef = useRef<string | null>(null);
  selectedRef.current = selectedId;

  useEffect(() => {
    const unlisten = [
      onCoreEvent((e) => {
        if (e.type === "sessionUpdated") {
          const prev = prevStatus.current.get(e.session.id);
          prevStatus.current.set(e.session.id, e.session.status);
          setSessions((list) => sortSessions([e.session, ...list.filter((x) => x.id !== e.session.id)]));
          void maybeNotify(e.session, prev);
        } else if (e.type === "sessionRemoved") {
          setSessions((list) => list.filter((x) => x.id !== e.id));
        } else {
          void api.listFolders().then(setFolders, () => {});
        }
      }),
      onResync(() => {
        void reload().catch(() => {});
        void api.agents().then(setAgents, () => {});
      }),
      onDaemonStatus(setDaemon),
      getCurrentWindow().onFocusChanged(({ payload }) => {
        focused.current = payload;
        if (payload && notifiedFocus.current) {
          setSelectedId(notifiedFocus.current);
          setView("focus");
          setTab("terminal");
          notifiedFocus.current = null;
        }
      }),
    ];
    void api.daemonStatus().then((s) => {
      setDaemon(s);
      if (s.connected) {
        void reload().catch(() => {});
        void api.agents().then(setAgents, () => {});
      }
    });
    return () => unlisten.forEach((p) => p.then((f) => f()));
  }, [reload, maybeNotify]);

  // Dock badge: sessions waiting on you.
  const needCount = sessions.filter((s) => s.status === "needs_you").length;
  useEffect(() => {
    void getCurrentWindow()
      .setBadgeCount(needCount > 0 ? needCount : undefined)
      .catch(() => {});
  }, [needCount]);

  // Keep a valid selection.
  useEffect(() => {
    if (selectedId && !sessions.some((s) => s.id === selectedId)) setSelectedId(null);
    if (splitId && !sessions.some((s) => s.id === splitId)) setSplitId(null);
    if (!selectedId && sessions.length > 0) {
      setSelectedId((sessions.find((s) => s.live) ?? sessions[0]).id);
    }
  }, [sessions, selectedId, splitId]);

  const addFolder = useCallback(
    async (path: string) => {
      const f = await run(() => api.addFolder(path));
      if (f) setFolders((list) => (list.some((x) => x.id === f.id) ? list : [...list, f]));
      return f;
    },
    [run],
  );

  const home = useMemo(() => homeFrom(folders.map((f) => f.path)), [folders]);

  const store: Store = {
    folders,
    sessions,
    agents,
    daemon,
    home,
    selectedId,
    splitId,
    view,
    tab,
    newSession,
    paletteOpen,
    handOffFor,
    broadcastFor,
    envFolder,
    setEnvFolder,
    renameFor,
    subagentView,
    settingsOpen,
    confirmRequest,
    inspectorOpen,
    toasts,
    select(id, opts) {
      setSelectedId(id);
      if (id && id === splitId) setSplitId(null);
      if (opts?.view) setView(opts.view);
      if (opts?.tab) setTab(opts.tab);
    },
    setSplit(id) {
      setSplitId(id && id !== selectedId ? id : null);
    },
    setView,
    setTab,
    openNewSession: (o) => setNewSession(o ?? {}),
    closeNewSession: () => setNewSession(null),
    setPaletteOpen,
    setHandOffFor,
    setBroadcastFor,
    setRenameFor,
    setSubagentView,
    setSettingsOpen,
    confirm: setConfirmRequest,
    toggleInspector() {
      const next = !inspectorOpen;
      setInspectorOpen(next);
      try {
        localStorage.setItem(INSPECTOR_KEY, next ? "open" : "closed");
      } catch {
        // Storage unavailable: the toggle still works for this window.
      }
    },
    toast,
    dismissToast: (id) => setToasts((t) => t.filter((x) => x.id !== id)),
    run,
    async moveFolder(id, before) {
      await run(async () => {
        await api.moveFolder(id, before);
        setFolders(await api.listFolders());
      });
    },
    restarting,
    terminalEpochs,
    async restartSession(id) {
      if (restartPending.current.has(id)) return;
      restartPending.current.add(id);
      setRestarting(new Set(restartPending.current));
      try {
        await run(async () => {
          const session = sessions.find((s) => s.id === id);
          const canResume = agents?.find((a) => a.agent === session?.agent)?.canResume ?? false;
          const resumed = await restartSession(id, api, canResume);
          setSessions((list) => sortSessions([resumed, ...list.filter((s) => s.id !== id)]));
          // Force a fresh attachment even if React batches the stopped/running events.
          setTerminalEpochs((epochs) => ({ ...epochs, [id]: (epochs[id] ?? 0) + 1 }));
          setSelectedId(id);
          if (splitId === id) setSplitId(null);
          setView("focus");
          setTab("terminal");
          toast("Session restarted", "info");
        });
      } finally {
        restartPending.current.delete(id);
        setRestarting(new Set(restartPending.current));
      }
    },
    addFolder,
    refreshAgents,
    folderOf: (s) => folders.find((f) => f.id === s.folderId),
  };

  return <Ctx.Provider value={store}>{children}</Ctx.Provider>;
}

function sortSessions(list: Session[]): Session[] {
  return [...list].sort((a, b) => b.createdAt - a.createdAt);
}
