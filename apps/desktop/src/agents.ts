import type { AgentId, PermissionPreset, Session, SessionStatus } from "./api";

/** Agent identity: a tinted tile holding the maker's mark (see AgentLogo); `mono` is the
 *  two-letter text form, used where only text fits. */
export const AGENTS: Record<AgentId, { name: string; mono: string; bg: string; fg: string }> = {
  claude: { name: "Claude Code", mono: "CC", bg: "#3A2219", fg: "#F0A584" },
  codex: { name: "Codex CLI", mono: "CX", bg: "#123026", fg: "#5FD3A8" },
  vibe: { name: "Mistral Vibe", mono: "MV", bg: "#3A2410", fg: "#FFA463" },
  antigravity: { name: "Antigravity", mono: "AG", bg: "#172744", fg: "#8DB6FF" },
};

export const AGENT_ORDER: AgentId[] = ["claude", "codex", "vibe", "antigravity"];

/** Status is always a coloured dot plus a text label. */
export const STATUS: Record<SessionStatus, { label: string; color: string; text: string }> = {
  needs_you: { label: "Needs you", color: "#F5A524", text: "#F5C26B" },
  running: { label: "Running", color: "#5B9BFF", text: "#9AA1AC" },
  idle: { label: "Idle", color: "#8B929C", text: "#9AA1AC" },
  done: { label: "Done", color: "#3FB950", text: "#9AA1AC" },
  error: { label: "Error", color: "#F85149", text: "#FF8A80" },
};

export const PERMISSIONS: { key: PermissionPreset; label: string; desc: string }[] = [
  { key: "ask_first", label: "Ask first", desc: "Approve every edit and command" },
  { key: "auto_edit", label: "Auto-edit", desc: "Edits OK, ask before commands" },
  { key: "full_auto", label: "Full auto", desc: "No prompts. Use in a sandbox" },
];

const DEFAULT_PERMISSION_KEY = "termlab.defaultPermission";

/** The permissions a new session starts with, as chosen in Settings ("Ask first" until then). */
export function defaultPermission(): PermissionPreset {
  try {
    const saved = localStorage.getItem(DEFAULT_PERMISSION_KEY);
    return PERMISSIONS.find((p) => p.key === saved)?.key ?? "ask_first";
  } catch {
    return "ask_first";
  }
}

export function setDefaultPermission(p: PermissionPreset) {
  try {
    localStorage.setItem(DEFAULT_PERMISSION_KEY, p);
  } catch {
    // Storage unavailable: the choice lasts for this window only.
  }
}

export function permissionLabel(p: PermissionPreset | null): string {
  return PERMISSIONS.find((x) => x.key === p)?.label ?? "—";
}

export function needsAttention(s: Session): boolean {
  return s.status === "needs_you" || s.status === "error";
}

export function relativeTime(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 10) return "now";
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

export function whenText(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const today = new Date(now);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === today.toDateString()) return `today ${time}`;
  const y = new Date(now - 86_400_000);
  if (d.toDateString() === y.toDateString()) return `yesterday ${time}`;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return String(n);
}

export function formatCost(usd: number): string {
  return usd < 10 ? `$${usd.toFixed(2)}` : `$${usd.toFixed(1)}`;
}

/** Replace the home directory prefix with ~ for display. */
export function tildify(path: string, home: string | null): string {
  if (home && (path === home || path.startsWith(home + "/"))) return "~" + path.slice(home.length);
  return path;
}

export function homeFrom(paths: string[]): string | null {
  for (const p of paths) {
    const m = /^(\/Users\/[^/]+|\/home\/[^/]+)/.exec(p);
    if (m) return m[1];
  }
  return null;
}

/** Session colours (Claude's /color names), tuned to read on every dark theme. */
const SESSION_COLORS: Record<string, string> = {
  red: "#f2716b",
  blue: "#6aa8ff",
  green: "#7fd17a",
  yellow: "#e8c065",
  purple: "#b58cf5",
  orange: "#f0a45d",
  pink: "#f28cc0",
  cyan: "#5fcfd6",
};

/** Class and style that mark an element with a session's colour, if it has one. */
export function sessionColor(color: string | null | undefined): { className: string; style?: Record<string, string> } {
  const hex = color ? SESSION_COLORS[color] : undefined;
  return hex ? { className: " has-color", style: { "--session-color": hex } } : { className: "" };
}
