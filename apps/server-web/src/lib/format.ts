import type { SessionStatus } from '@/types';

const AGENTS: Record<string, { name: string; short: string }> = {
    claude: { name: 'Claude Code', short: 'claude' },
    codex: { name: 'Codex CLI', short: 'codex' },
    vibe: { name: 'Mistral Vibe', short: 'vibe' },
    agy: { name: 'Antigravity CLI', short: 'agy' },
    antigravity: { name: 'Antigravity CLI', short: 'agy' },
    shell: { name: 'Terminal', short: 'shell' },
};

export function agentName(agent: string): string {
    return AGENTS[agent]?.name ?? agent;
}

export function agentShort(agent: string): string {
    return AGENTS[agent]?.short ?? agent;
}

export const STATUS_LABEL: Record<SessionStatus, string> = {
    needs_you: 'Needs you',
    running: 'Running',
    idle: 'Idle',
    done: 'Done',
    error: 'Error',
};

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
