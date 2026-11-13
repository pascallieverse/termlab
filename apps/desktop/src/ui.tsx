// Small shared UI pieces: monograms, status dots and pills, icons.
import { AgentLogo } from "./AgentLogo";

import { useId, type ReactNode } from "react";
import type { AgentId, Session, SessionStatus } from "./api";
import { AGENTS, STATUS } from "./agents";

export function Monogram({ agent, size = 28 }: { agent: AgentId; size?: number }) {
  const a = AGENTS[agent];
  return (
    <span
      className="monogram"
      style={{
        background: a.bg,
        color: a.fg,
        width: size,
        height: size,
        borderRadius: size >= 36 ? 9 : size >= 32 ? 8 : 7,
      }}
      title={a.name}
      aria-hidden="true"
    >
      <AgentLogo agent={agent} size={Math.round(size * 0.62)} />
    </span>
  );
}

export function Dot({ status }: { status: SessionStatus }) {
  return <span className="dot" style={{ background: STATUS[status].color }} aria-hidden="true" />;
}

export function StatusText({ status }: { status: SessionStatus }) {
  return <span style={{ color: STATUS[status].text }}>{STATUS[status].label}</span>;
}

export function StatusPill({ session, compact }: { session: Session; compact?: boolean }) {
  const st = STATUS[session.status];
  const strong = session.status === "needs_you" || session.status === "error";
  return (
    <span
      className={`pill${compact ? " compact" : ""}`}
      style={{ borderColor: st.color + "55", color: strong ? st.text : "var(--text-2)" }}
    >
      <Dot status={session.status} />
      {st.label}
      {session.status === "error" && session.exitCode !== null && session.exitCode !== -1
        ? ` · exit ${session.exitCode}`
        : ""}
    </span>
  );
}

export function Icon({ name, size = 14 }: { name: IconName; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {ICONS[name]}
    </svg>
  );
}

type IconName =
  | "folder"
  | "folderPlus"
  | "plus"
  | "search"
  | "close"
  | "arrow"
  | "stop"
  | "lines"
  | "grid"
  | "focus"
  | "fork"
  | "terminal"
  | "warning"
  | "refresh"
  | "trash"
  | "settings"
  | "chevron"
  | "panel"
  | "music"
  | "volume"
  | "volumeOff"
  | "expand"
  | "play"
  | "pause"
  | "prev"
  | "next";

const ICONS: Record<IconName, ReactNode> = {
  folder: <path d="M3 6h6l2 2h10v11H3z" />,
  folderPlus: <path d="M3 6h6l2 2h10v11H3zM12 11v5M9.5 13.5h5" />,
  plus: <path d="M12 5v14M5 12h14" />,
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </>
  ),
  close: <path d="M6 6l12 12M18 6 6 18" />,
  arrow: <path d="M5 12h14M13 6l6 6-6 6" />,
  stop: <rect x="6" y="6" width="12" height="12" rx="2" />,
  lines: <path d="M4 12h16M4 6h16M4 18h10" />,
  grid: (
    <>
      <rect x="4" y="4" width="7" height="7" rx="1" />
      <rect x="13" y="4" width="7" height="7" rx="1" />
      <rect x="4" y="13" width="7" height="7" rx="1" />
      <rect x="13" y="13" width="7" height="7" rx="1" />
    </>
  ),
  focus: <rect x="4" y="5" width="16" height="14" rx="2" />,
  fork: (
    <>
      <circle cx="6" cy="5" r="2" />
      <circle cx="18" cy="5" r="2" />
      <circle cx="12" cy="19" r="2" />
      <path d="M6 7v2a4 4 0 0 0 4 4h4a4 4 0 0 0 4-4V7M12 13v4" />
    </>
  ),
  terminal: <path d="m5 8 4 4-4 4M12 16h7" />,
  warning: <path d="M12 4 2 20h20L12 4zM12 10v4M12 17v.5" />,
  refresh: <path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7" />,
  trash: <path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13" />,
  chevron: <path d="m9 6 6 6-6 6" />,
  music: (
    <>
      <path d="M9 18V5l11-2v13" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="17" cy="16" r="3" />
    </>
  ),
  expand: <path d="M14 4h6v6M10 20H4v-6M20 4l-7 7M4 20l7-7" />,
  volume: (
    <>
      <path d="M4 9.5v5h3.5l4.5 4v-13l-4.5 4H4z" fill="currentColor" stroke="none" />
      <path d="M15.5 9a4.2 4.2 0 0 1 0 6M18 6.5a8 8 0 0 1 0 11" />
    </>
  ),
  volumeOff: (
    <>
      <path d="M4 9.5v5h3.5l4.5 4v-13l-4.5 4H4z" fill="currentColor" stroke="none" />
      <path d="m16 9.5 5 5M21 9.5l-5 5" />
    </>
  ),
  play: <path d="M7 4.5v15l12.5-7.5z" fill="currentColor" />,
  pause: <path d="M7 5h3.5v14H7zM13.5 5H17v14h-3.5z" fill="currentColor" />,
  prev: <path d="M6 5v14M19 5.5v13L9 12z" fill="currentColor" />,
  next: <path d="M18 5v14M5 5.5v13L15 12z" fill="currentColor" />,
  panel: (
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M15 4v16" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
    </>
  ),
};

/** The Termlab mark: a lab flask holding a terminal prompt. Same drawing as `app-icon.svg`. */
export function Logo() {
  const mask = useId();
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="var(--accent)" stroke="var(--accent)" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <mask id={mask} maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24">
        <rect width="24" height="24" fill="#fff" stroke="none" />
        <path d="M8.9 17l2.3 1.9-2.3 1.9M13 20.8h2.8" fill="none" stroke="#000" strokeWidth="1.8" />
      </mask>
      <circle cx="13.7" cy="5.4" r="1.8" stroke="none" />
      <circle cx="16.5" cy="2.1" r="1.1" stroke="none" />
      <g mask={`url(#${mask})`}>
        <path d="M10.1 9.4v3.1L3.9 22.2h16.2l-6.2-9.7V9.4z" strokeWidth="1.5" />
        <path d="M8.9 9.2h6.2" strokeWidth="2" />
      </g>
    </svg>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="kbd">{children}</kbd>;
}
