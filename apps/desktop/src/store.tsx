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
  type Label,
  type LabelBook,
  type Session,
} from "./api";
import { NO_LABELS, labelsBySession, withSessionLabels } from "./sessionLabels";
import { AGENTS, STATUS, homeFrom } from "./agents";
import { matchesSessionFilter, type SessionFilter } from "./sessionFilters";
import { getPrefs, usePrefs } from "./prefs";

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
  visibleSessions: Session[];
  sessionFilter: SessionFilter;
  setSessionFilter(filter: SessionFilter): void;
  setOnHold(id: string, onHold: boolean): Promise<void>;
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
  /** Every label, in the order they were made. */
  labels: Label[];
  /** A session's labels. */
  labelsOf(sessionId: string): Label[];
  /** The session whose labels are being chosen. */
  labelsFor: string | null;
  setLabelsFor(id: string | null): void;
  /** Makes a label (`id` null) or changes one; answers with it as saved. */
  saveLabel(id: number | null, name: string, color: string): Promise<Label | undefined>;
  /** Removes a label, from every session too. */
  deleteLabel(id: number): Promise<void>;
  setSessionLabels(sessionId: string, ids: number[]): Promise<void>;
  /** The sub-agent whose steps are open. */
  subagentView: { sessionId: string; subagentId: string } | null;
  settingsOpen: boolean;
  confirmRequest: ConfirmRequest | null;
  /** The session inspector (right column); closed by default, remembered per viewer. */
  inspectorOpen: boolean;
  /** What the right column shows: the session's details or its project's notes. */
  inspectorTab: InspectorTab;
  toasts: Toast[];

  select(id: string | null, opts?: { view?: View; tab?: Tab }): void;
  /**
   * Opens a session the app just created (started, resumed, forked, handed off). Its answer
   * can arrive before the update that adds it to the list, so it's added here first:
   * selecting a session the list doesn't know falls back to another one.
   */
  show(session: Session, opts?: { view?: View; tab?: Tab }): void;
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
  /** Opens the right column on a tab; closes it when it already shows that tab. */
  showInspector(tab: InspectorTab): void;
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
const INSPECTOR_TAB_KEY = "termlab.inspector.tab";

export type InspectorTab = "session" | "notes";

/** The inspector's state is a per-viewer convenience, kept in localStorage. */
function remember(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage unavailable: it still works for this window.
  }
}

const NONE: Label[] = [];

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
  const [sessionFilter, setSessionFilter] = useState<SessionFilter>("all");
  const visibleSessions = useMemo(
    () => sessions.filter((s) => matchesSessionFilter(s, sessionFilter === "on_hold" ? "on_hold" : "all")),
    [sessions, sessionFilter],
  );
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
  const [labelBook, setLabelBook] = useState<LabelBook>(NO_LABELS);
  const labelBookRef = useRef(labelBook);
  labelBookRef.current = labelBook;
  const sessionLabels = useMemo(() => labelsBySession(labelBook), [labelBook]);
  // The same function until the labels change, so lists that filter by label can keep theirs.
  const labelsOf = useCallback((sessionId: string) => sessionLabels.get(sessionId) ?? NONE, [sessionLabels]);
  const [labelsFor, setLabelsFor] = useState<string | null>(null);
  const labelSaves = useRef(0);
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
  const [inspectorTab, setInspectorTabState] = useState<InspectorTab>(() => {
    try {
      return localStorage.getItem(INSPECTOR_TAB_KEY) === "notes" ? "notes" : "session";
    } catch {
      return "session";
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
    if (s.onHold || !s.notify || prev === s.status || prev === undefined) return;
    const needs = s.status === "needs_you";
    const finished = (s.status === "done" || s.status === "error") && prev !== "done" && prev !== "error";
    if (!needs && !finished) return;
    const prefs = getPrefs();
    if (needs ? !prefs.notifyNeedsYou : !prefs.notifyFinished) return;
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
          if (e.session.onHold && notifiedFocus.current === e.session.id) notifiedFocus.current = null;
          const prev = prevStatus.current.get(e.session.id);
          prevStatus.current.set(e.session.id, e.session.status);
          setSessions((list) => sortSessions([e.session, ...list.filter((x) => x.id !== e.session.id)]));
          void maybeNotify(e.session, prev);
        } else if (e.type === "sessionRemoved") {
          setSessions((list) => list.filter((x) => x.id !== e.id));
          // Its labels go with it.
          if (labelBookRef.current.sessions[e.id]) void api.setSessionLabels(e.id, []).then(setLabelBook, () => {});
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
          setSessionFilter("all");
          setView("focus");
          setTab("terminal");
          notifiedFocus.current = null;
        }
      }),
    ];
    // Labels are the app's own: they don't wait for the background service.
    void api.readLabels().then(setLabelBook, (e: unknown) => toast(errorText(e)));
    void api.daemonStatus().then((s) => {
      setDaemon(s);
      if (s.connected) {
        void reload().catch(() => {});
        void api.agents().then(setAgents, () => {});
      }
    });
    return () => unlisten.forEach((p) => p.then((f) => f()));
  }, [reload, maybeNotify, toast]);

  // Dock badge: sessions waiting on you.
  const { dockBadge } = usePrefs();
  const needCount = sessions.filter((s) => !s.onHold && s.status === "needs_you").length;
  useEffect(() => {
    void getCurrentWindow()
      .setBadgeCount(dockBadge && needCount > 0 ? needCount : undefined)
      .catch(() => {});
  }, [needCount, dockBadge]);

  // Keep a valid selection.
  useEffect(() => {
    if (!visibleSessions.some((s) => s.id === selectedId)) {
      setSelectedId((visibleSessions.find((s) => s.live) ?? visibleSessions[0])?.id ?? null);
    }
    if (splitId && !visibleSessions.some((s) => s.id === splitId)) setSplitId(null);
  }, [visibleSessions, selectedId, splitId]);

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
    visibleSessions,
    sessionFilter,
    setSessionFilter,
    async setOnHold(id, onHold) {
      await run(async () => {
        await api.setOnHold(id, onHold);
        const updated = await api.session(id);
        setSessions((list) => sortSessions(list.map((s) => s.id === id ? updated : s)));
      });
    },
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
    labels: labelBook.labels,
    labelsOf,
    labelsFor,
    setLabelsFor,
    async saveLabel(id, name, color) {
      const saved = await run(() => api.saveLabel(id, name, color));
      if (saved) setLabelBook(saved.book);
      return saved?.label;
    },
    async deleteLabel(id) {
      const book = await run(() => api.deleteLabel(id));
      if (book) setLabelBook(book);
    },
    async setSessionLabels(sessionId, ids) {
      // Shown at once, so a second change made before this one is saved builds on it.
      setLabelBook((book) => withSessionLabels(book, sessionId, ids));
      const save = ++labelSaves.current;
      try {
        const book = await api.setSessionLabels(sessionId, ids);
        // An answer that a newer change has overtaken would undo it on screen for a moment.
        if (save === labelSaves.current) setLabelBook(book);
      } catch (e) {
        toast(errorText(e));
        // What's shown may not be what's saved: show what is.
        void api.readLabels().then(setLabelBook, () => {});
      }
    },
    subagentView,
    settingsOpen,
    confirmRequest,
    inspectorOpen,
    inspectorTab,
    toasts,
    select(id, opts) {
      const session = sessions.find((s) => s.id === id);
      if (session?.onHold) setSessionFilter("on_hold");
      else if (session && sessionFilter === "on_hold") setSessionFilter("all");
      setSelectedId(id);
      if (id && id === splitId) setSplitId(null);
      if (opts?.view) setView(opts.view);
      if (opts?.tab) setTab(opts.tab);
    },
    show(session, opts) {
      if (session.onHold) setSessionFilter("on_hold");
      else if (sessionFilter === "on_hold") setSessionFilter("all");
      // Only when it's missing: an update that already arrived is newer than this answer.
      setSessions((list) => (list.some((x) => x.id === session.id) ? list : sortSessions([session, ...list])));
      if (!prevStatus.current.has(session.id)) prevStatus.current.set(session.id, session.status);
      setSelectedId(session.id);
      if (session.id === splitId) setSplitId(null);
      setView(opts?.view ?? "focus");
      setTab(opts?.tab ?? "terminal");
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
      remember(INSPECTOR_KEY, next ? "open" : "closed");
    },
    showInspector(tab) {
      const next = !(inspectorOpen && inspectorTab === tab);
      setInspectorOpen(next);
      remember(INSPECTOR_KEY, next ? "open" : "closed");
      if (next) {
        setInspectorTabState(tab);
        remember(INSPECTOR_TAB_KEY, tab);
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
