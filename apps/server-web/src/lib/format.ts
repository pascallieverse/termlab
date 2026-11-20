import type { SessionStatus } from '@/types';

/** Agent identity, as in the desktop app (apps/desktop/src/agents.ts): a tinted tile holding the
 *  maker's mark (see AgentLogo). */
export type AgentId = 'claude' | 'codex' | 'vibe' | 'antigravity' | 'shell';

export const AGENTS: Record<AgentId, { name: string; short: string; bg: string; fg: string }> = {
    claude: { name: 'Claude Code', short: 'claude', bg: '#3A2219', fg: '#F0A584' },
    codex: { name: 'Codex CLI', short: 'codex', bg: '#123026', fg: '#5FD3A8' },
    vibe: { name: 'Mistral Vibe', short: 'vibe', bg: '#3A2410', fg: '#FFA463' },
    antigravity: { name: 'Antigravity', short: 'agy', bg: '#172744', fg: '#8DB6FF' },
    shell: { name: 'Terminal', short: 'shell', bg: '#24272D', fg: '#C9CED6' },
};

/** The agent the server reported, as one of the known ones (unknown agents show as a terminal). */
export function agentId(agent: string): AgentId {
    if (agent === 'agy') {
        return 'antigravity';
    }

    return agent in AGENTS ? (agent as AgentId) : 'shell';
}

export function agentName(agent: string): string {
    return agent in AGENTS || agent === 'agy' ? AGENTS[agentId(agent)].name : agent;
}

export function agentShort(agent: string): string {
    return agent in AGENTS || agent === 'agy' ? AGENTS[agentId(agent)].short : agent;
}

/** Status is always a coloured dot plus a text label (the desktop's STATUS). */
export const STATUS: Record<SessionStatus, { label: string; color: string; text: string }> = {
    needs_you: { label: 'Needs you', color: '#F5A524', text: '#F5C26B' },
    running: { label: 'Running', color: '#5B9BFF', text: '#9AA1AC' },
    idle: { label: 'Idle', color: '#8B929C', text: '#9AA1AC' },
    done: { label: 'Done', color: '#3FB950', text: '#9AA1AC' },
    error: { label: 'Error', color: '#F85149', text: '#FF8A80' },
};

export const STATUS_LABEL: Record<SessionStatus, string> = {
    needs_you: 'Needs you',
    running: 'Running',
    idle: 'Idle',
    done: 'Done',
    error: 'Error',
};

/** "now", "45s", "3m", "2h", "4d": the desktop's short relative time. */
export function relativeTime(iso: string | null | undefined, now = Date.now()): string {
    if (!iso) {
        return '';
    }
    const s = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000));
    if (s < 10) {
        return 'now';
    }
    if (s < 60) {
        return `${s}s`;
    }
    const m = Math.round(s / 60);
    if (m < 60) {
        return `${m}m`;
    }
    const h = Math.round(m / 60);
    if (h < 24) {
        return `${h}h`;
    }

    return `${Math.round(h / 24)}d`;
}

/** "2 min ago", "3 h ago", "Yesterday", "12 Nov". */
export function timeAgo(iso: string | null | undefined): string {
    if (!iso) {
        return '—';
    }

    const seconds = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);

    if (seconds < 45) {
        return 'just now';
    }

    if (seconds < 3600) {
        return `${Math.round(seconds / 60)} min ago`;
    }

    if (seconds < 86400) {
        return `${Math.round(seconds / 3600)} h ago`;
    }

    if (seconds < 172800) {
        return 'Yesterday';
    }

    return formatDate(iso);
}

export function formatDate(iso: string | null | undefined): string {
    if (!iso) {
        return '—';
    }

    return new Date(iso).toLocaleDateString(undefined, {
        day: 'numeric',
        month: 'short',
        year: 'numeric',
    });
}

/** Whether Termlab version `v` (e.g. "0.2.10") is at least `min`. */
export function atLeast(v: string | null | undefined, min: string): boolean {
    if (!v) return false;
    const a = v.split(/[.+-]/).map((x) => parseInt(x, 10) || 0);
    const b = min.split('.').map((x) => parseInt(x, 10) || 0);
    for (let i = 0; i < 3; i++) {
        if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0);
    }
    return true;
}

export function compactNumber(n: number | null | undefined): string {
    if (n === null || n === undefined) {
        return '—';
    }

    return new Intl.NumberFormat(undefined, {
        notation: 'compact',
        maximumFractionDigits: 0,
    })
        .format(n)
        .toLowerCase();
}
