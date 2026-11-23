// Typed access to the Termlab daemon (through the Tauri commands in src-tauri/src).

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type AgentId = "claude" | "codex" | "vibe" | "antigravity" | "shell";
export type SessionStatus = "needs_you" | "running" | "idle" | "done" | "error";
export type Billing = "subscription" | "api_key" | "unknown";
export type PermissionPreset = "ask_first" | "auto_edit" | "full_auto";
export type ApprovalChoice = "allow_once" | "allow_always" | "deny";

export interface Folder {
  id: number;
  name: string;
  path: string;
  addedAt: number;
  sortOrder: number;
}

/** A project's notes, shared by all of its sessions (kept by the app, see project_notes.rs). */
export interface ProjectNotes {
  text: string;
  /** When they were last saved; null when there are none. */
  updatedAt: number | null;
}

export interface ApprovalOption {
  number: number;
  label: string;
  choice: ApprovalChoice | null;
}

export interface Approval {
  /** A permission prompt (answered with the buttons) or a question of the agent's own. */
  kind: "permission" | "question";
  question: string;
  detail: string | null;
  options: ApprovalOption[];
}

export interface Usage {
  contextTokens: number;
  contextWindow: number | null;
  totalTokens: number;
  costUsd: number | null;
  model: string | null;
}

/** One limit of the plan: a rolling window (five_hour, seven_day, …) and how much is used. */
export interface UsageWindow {
  id: string;
  /** A name for windows the UI doesn't know by id, e.g. "Current week (Fable)". */
  label?: string | null;
  usedPercentage: number;
  resetsAt: number | null;
}

export interface PlanUsage {
  agent: AgentId;
  windows: UsageWindow[];
  updatedAt: number;
  /** Whether the agent is signed in to its service, when known. */
  connected?: boolean | null;
  /** The plan's name as the service gives it. */
  plan?: string | null;
  /** Something to say instead of percentages, e.g. why there are none. */
  note?: string | null;
}

/** A sub-agent the agent started inside a session (Claude's Agent tool, Codex's spawned agents). */
export interface SubAgent {
  id: string;
  name: string;
  description: string | null;
  model: string | null;
  running: boolean;
  startedAt: number | null;
  lastActivityAt: number;
}

export interface Session {
  id: string;
  folderId: number;
  agent: AgentId;
  title: string;
  model: string | null;
  permissionMode: PermissionPreset | null;
  isolation: "checkout";
  agentSessionId: string | null;
  status: SessionStatus;
  exitCode: number | null;
  createdAt: number;
  lastActivityAt: number;
  billing: Billing;
  startCommit: string | null;
  transcriptPath: string | null;
  notify: boolean;
  onHold: boolean;
  origin: "termlab" | "imported" | "attached";
  /** A colour set inside the agent (Claude's /color): red, blue, green, yellow, purple, orange, pink or cyan. */
  color?: string | null;
  live: boolean;
  preview: string[];
  approval: Approval | null;
  usage: Usage | null;
  contextPercent: number | null;
  apiKeyVars: string[];
  /** Sub-agents of a running session: working now, or just finished. */
  subagents?: SubAgent[];
  /** A different version of the agent was installed after this session started; restarting runs it. */
  updateReady?: boolean;
}

export interface Installed {
  agent: AgentId;
  path: string;
  version: string | null;
  loginBilling: Billing;
}

export interface AgentInfo {
  agent: AgentId;
  name: string;
  binary: string;
  installed: Installed | null;
  modelFlag: boolean;
  modelSuggestions: string[];
  canResume: boolean;
  canFork: boolean;
  hooks: boolean;
  verified: boolean;
}

export interface ChangedFile {
  path: string;
  added: number | null;
  deleted: number | null;
  untracked: boolean;
}

export interface SessionDetails {
  cwd: string;
  branch: string | null;
  files: ChangedFile[];
  usage: Usage | null;
  contextPercent: number | null;
  resumeCommand: string | null;
  canFork: boolean;
  canResume: boolean;
}

export interface TimelineEntry {
  at: number;
  kind: "prompt" | "tool" | "message" | "hook";
  text: string;
}

export interface PastSession {
  agent: AgentId;
  agentSessionId: string;
  title: string;
  cwd: string;
  lastActivityAt: number;
  path: string;
}

export interface Discovered extends PastSession {
  /** Set when the folder has been added to Termlab. */
  folderId: number | null;
  termlabSession: string | null;
  /** The agent is running this session right now, outside Termlab. */
  runningElsewhere: boolean;
}

export interface ExternalSession {
  agent: AgentId;
  agentSessionId: string;
  shortId: string | null;
  name: string | null;
  cwd: string;
  kind: "background" | "interactive";
  status: SessionStatus | null;
  startedAt: number;
  folderId: number | null;
  attachable: boolean;
  termlabSession: string | null;
}

export interface ImportSummary {
  imported: number;
  alreadyPresent: number;
  foldersAdded: number;
  skipped: number;
}

export interface TermSize {
  cols: number;
  rows: number;
}

export type StartMode =
  | { kind: "new" }
  | { kind: "resume"; agentSessionId: string }
  | { kind: "fork"; agentSessionId: string };

export interface StartRequest {
  folderId: number;
  agent: AgentId;
  model?: string | null;
  permission: PermissionPreset;
  firstMessage?: string | null;
  size?: TermSize | null;
  notify?: boolean;
  title?: string | null;
  mode?: StartMode;
}

export type CoreEvent =
  | { type: "sessionUpdated"; session: Session }
  | { type: "sessionRemoved"; id: string }
  | { type: "foldersChanged" };

/** What a look for a newer Termlab found. */
export interface UpdateCheck {
  /** False in builds that weren't set up for updates (development builds among them). */
  supported: boolean;
  currentVersion: string;
  /** The newer version, when there is one. */
  version: string | null;
  notes: string | null;
}

export interface DaemonStatus {
  connected: boolean;
  message: string | null;
  /** Connected to a background service from an older build, kept for its sessions. */
  outdated?: boolean;
}

/** Remote access: this computer linked to your own Termlab server (see termlabd's remote.rs). */
export type RemoteState = "unpaired" | "connecting" | "connected" | "no_plan" | "error";

export interface RemoteStatus {
  state: RemoteState;
  serverUrl: string | null;
  machineName: string | null;
  accountName: string | null;
  plan: string | null;
  error: string | null;
  /** Sessions this computer currently shows remotely. */
  sessions: number;
  /** Browsers watching a terminal right now. */
  viewers: number;
  defaultMachineName: string;
}

export type ScreenStatus = {
  supported: boolean;
  enabled: boolean;
  /** Ask on this computer before each session. */
  confirm: boolean;
  screenAllowed: boolean;
  inputAllowed: boolean;
  /** The session running now. */
  active: { pid: number; startedAt: number; viewer: string } | null;
};

function call<T>(method: string, params?: unknown): Promise<T> {
  const request = params === undefined ? { method } : { method, params };
  return invoke<T>("daemon_call", { request });
}

export const api = {
  readProjectEnv: (cwd: string) => invoke<string | null>("read_project_env", { cwd }),
  saveProjectEnv: (cwd: string, content: string, original: string | null) =>
    invoke<null>("save_project_env", { cwd, content, original }),
  moveFolder: (id: number, before: number | null) => call<null>("moveFolder", { id, before }),
  listFolders: () => call<Folder[]>("listFolders"),
  addFolder: (path: string) => call<Folder>("addFolder", { path }),
  removeFolder: (id: number) => call<null>("removeFolder", { id }),
  /** A project's notes, by its folder's path. */
  readProjectNotes: (project: string) => invoke<ProjectNotes>("read_project_notes", { project }),
  /** Remote screen sharing: the switch, macOS permissions and the session running now. */
  screenStatus: () => invoke<ScreenStatus>("screen_status"),
  screenSet: (enabled: boolean, confirm: boolean) => invoke<ScreenStatus>("screen_set", { enabled, confirm }),
  screenRequestPermission: (kind: "screen" | "input") => invoke<null>("screen_request_permission", { kind }),
  screenStop: () => invoke<null>("screen_stop"),
  /** Replaces a project's notes; blank text removes them. */
  saveProjectNotes: (project: string, text: string) =>
    invoke<ProjectNotes>("save_project_notes", { project, text }),
  agents: () => call<AgentInfo[]>("agents"),
  /** Each agent's subscription plan limits, as last reported (none: an empty list). */
  planUsage: () => call<unknown>("planUsage").then(asUsageList),
  /** Asks the agents for fresh limits in the background; answers with the current ones. */
  refreshPlanUsage: () => call<unknown>("refreshPlanUsage").then(asUsageList),
  refreshAgents: () => call<AgentInfo[]>("refreshAgents"),
  session: (id: string) => call<Session>("session", { id }),
  listSessions: () => call<Session[]>("listSessions"),
  previewCommand: (request: StartRequest, id: string) =>
    call<string>("previewCommand", { request: withDefaults(request), id }),
  startSession: (request: StartRequest, id?: string) =>
    call<Session>("startSession", { request: withDefaults(request), id: id ?? null }),
  restart: (id: string, size?: TermSize) => call<Session>("restart", { id, size: size ?? null }),
  /** Asks the agent to restart a background session it runs itself (one Termlab is attached to). */
  respawn: (id: string) => call<null>("respawn", { id }),
  fork: (id: string) => call<Session>("fork", { id, size: null }),
  handoffMessage: (id: string) => call<string>("handoffMessage", { id }),
  handOff: (id: string, agent: AgentId, permission: PermissionPreset, model: string | null) =>
    call<Session>("handOff", { id, agent, permission, model, size: null }),
  write: (id: string, data: string) => call<null>("write", { id, data }),
  writeBytes: (id: string, data: number[]) => call<null>("writeBytes", { id, data }),
  sendMessage: (id: string, text: string) => call<null>("sendMessage", { id, text }),
  broadcast: (folderId: number, text: string) => call<number>("broadcast", { folderId, text }),
  answer: (id: string, choice: ApprovalChoice) => call<null>("answer", { id, choice }),
  resize: (id: string, cols: number, rows: number) => call<null>("resize", { id, cols, rows }),
  stop: (id: string) => call<null>("stop", { id }),
  stopIdle: (folderId: number) => call<number>("stopIdle", { folderId }),
  deleteSession: (id: string) => call<null>("delete", { id }),
  setTitle: (id: string, title: string) => call<null>("setTitle", { id, title }),
  setOnHold: (id: string, onHold: boolean) => call<null>("setOnHold", { id, onHold }),
  details: (id: string) => call<SessionDetails>("details", { id }),
  diff: (id: string) => call<string>("diff", { id }),
  timeline: (id: string) => call<TimelineEntry[]>("timeline", { id }),
  /** Switches Claude's terminal to one of its sub-agents (null: back to the main conversation). */
  viewSubagent: (id: string, subagent: string | null) => call<null>("viewSubagent", { id, subagent }),
  /** What one of a session's sub-agents did. */
  subagentTimeline: (id: string, subagent: string) =>
    call<TimelineEntry[]>("subagentTimeline", { id, subagent }),
  discover: (folderId: number | null, everywhere = false) =>
    call<Discovered[]>("discover", { folderId, everywhere }),
  external: () => call<ExternalSession[]>("external"),
  importSessions: (items: PastSession[]) => call<ImportSummary>("import", { items }),
  attachExternal: (agent: AgentId, agentSessionId: string) =>
    call<Session>("attachExternal", { agent, agentSessionId, size: null }),

  /** Streams scrollback (after a terminal reset) then live output, as raw bytes. */
  attach(id: string, onData: (bytes: Uint8Array) => void) {
    const channel = new Channel<ArrayBuffer | number[]>();
    channel.onmessage = (msg) => onData(new Uint8Array(msg));
    return invoke<{ exited: { code: number; success: boolean } | null }>("attach_session", {
      id,
      onOutput: channel,
    });
  },
  detach: (id: string) => invoke<null>("detach_session", { id }),
  daemonStatus: () => invoke<DaemonStatus>("daemon_status"),
  /** Restarts an outdated background service; the new one resumes its sessions. */
  restartDaemon: () => invoke<null>("restart_daemon"),
  /**
   * Shows YouTube Music (music.youtube.com, in its own web view) over `rect` of the window,
   * in CSS pixels. `zoom` below 1 shrinks the page to fit the sidebar.
   */
  youtubeMusicDock: (rect: { x: number; y: number; width: number; height: number }, zoom: number) =>
    invoke<null>("youtube_music_dock", { rect, zoom }),
  /** Sets the docked page's volume, 0 to 100. */
  youtubeMusicVolume: (volume: number) => invoke<null>("youtube_music_volume", { volume }),
  /** Moves the docked page: back one page, to YouTube Music's home, or reloads it. */
  youtubeMusicNavigate: (action: "back" | "home" | "reload") =>
    invoke<null>("youtube_music_navigate", { action }),
  /** Hides it; the music keeps playing. */
  youtubeMusicHide: () => invoke<null>("youtube_music_hide"),
  /** What's playing there; null when it hasn't been opened or nothing has played. */
  youtubeMusicState: () => invoke<YouTubeMusicState | null>("youtube_music_state"),
  youtubeMusicControl: (action: "playPause" | "pause" | "next" | "previous") =>
    invoke<null>("youtube_music_control", { action }),
  /**
   * Plays a YouTube player address (www.youtube-nocookie.com/embed/…) in the stations' own
   * web view. It's out of view until placed.
   */
  stationOpen: (src: string) => invoke<null>("station_open", { src }),
  /** Places the stations' view over `rect` of the window (CSS pixels), or out of view for null; it keeps playing. */
  stationPlace: (rect: { x: number; y: number; width: number; height: number } | null) =>
    invoke<null>("station_place", { rect }),
  /** Sends the player a message of its postMessage protocol (JSON). */
  stationPost: (message: string) => invoke<null>("station_post", { message }),
  /** The player's messages since the last call (JSON strings), oldest first. */
  stationEvents: () => invoke<string[]>("station_events"),
  /** Stops the player. */
  stationStop: () => invoke<null>("station_stop"),
  /** The video a YouTube channel is live-streaming now (for the sidebar's music stations). */
  youtubeLiveVideo: (channel: string) => invoke<string | null>("youtube_live_video", { channel }),
  /** The computer's output volume, 0 to 100; null where it can't be read. */
  systemVolume: () => invoke<number | null>("system_volume"),
  setSystemVolume: (volume: number) => invoke<null>("set_system_volume", { volume }),
  /** Opens an https link in the user's browser. */
  openExternal: (url: string) => invoke<null>("open_external", { url }),
  remoteStatus: () => call<RemoteStatus>("remoteStatus"),
  remotePair: (serverUrl: string, code: string, machineName: string) =>
    call<RemoteStatus>("remotePair", { request: { serverUrl, code, machineName } }),
  remoteUnpair: () => call<RemoteStatus>("remoteUnpair"),
  updateCheck: () => invoke<UpdateCheck>("update_check"),
  updateInstall: () => invoke<null>("update_install"),
  openInTerminal: (cwd: string, command: string) =>
    invoke<null>("open_in_terminal", { cwd, command }),
};

function withDefaults(r: StartRequest): StartRequest {
  return {
    model: null,
    firstMessage: null,
    size: null,
    notify: true,
    title: null,
    mode: { kind: "new" },
    ...r,
  };
}

export function onCoreEvent(handler: (e: CoreEvent) => void): Promise<UnlistenFn> {
  return listen<CoreEvent>("core-event", (e) => handler(e.payload));
}

export function onResync(handler: () => void): Promise<UnlistenFn> {
  return listen("core-resync", () => handler());
}

export function onDaemonStatus(handler: (s: DaemonStatus) => void): Promise<UnlistenFn> {
  return listen<DaemonStatus>("daemon-status", (e) => handler(e.payload));
}

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

export interface YouTubeMusicState {
  title: string;
  artist: string;
  artwork: string | null;
  playing: boolean;
  /** The player's volume, 0 to 100 (0 when muted); null when unknown. */
  volume?: number | null;
}

/** An older background service answered with one object or null instead of a list. */
function asUsageList(value: unknown): PlanUsage[] {
  if (Array.isArray(value)) return value as PlanUsage[];
  return value ? [value as PlanUsage] : [];
}
