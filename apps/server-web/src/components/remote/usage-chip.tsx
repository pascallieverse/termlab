import { useEffect, useRef, useState } from 'react';
import { AgentLogo } from '@/components/el/agent';
import { AGENTS, agentId, type AgentId } from '@/lib/format';
import { useLive } from '@/live/LiveProvider';
import type { PlanUsage, UsageWindow } from '@/types';

// The subscription plans, as in the desktop app's top bar: per agent, how much of each limit
// is used. Termlab reports them from each machine; plan limits belong to the account, so the
// freshest report per agent is shown.

/** Older than this, the numbers are shown as possibly out of date. */
const STALE_MS = 30 * 60_000;
const ORDER: AgentId[] = ['claude', 'codex', 'vibe', 'antigravity'];

const LABELS: Record<string, string> = {
    five_hour: 'Current session (5 hours)',
    seven_day: 'Current week (all models)',
    seven_day_opus: 'Current week (Opus)',
    seven_day_sonnet: 'Current week (Sonnet)',
    spend_limit: 'Spend limit',
};

function label(w: UsageWindow, agent: string): string {
    if (w.id === 'seven_day' && agent !== 'claude') return 'Current week';
    return LABELS[w.id] ?? w.label ?? w.id.replace(/[_:]/g, ' ');
}

function level(percent: number): 'ok' | 'warn' | 'full' {
    return percent >= 95 ? 'full' : percent >= 80 ? 'warn' : 'ok';
}

function ago(ms: number): string {
    const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (s < 45) return 'just now';
    if (s < 3600) return `${Math.round(s / 60)}m ago`;
    if (s < 86_400) return `${Math.round(s / 3600)}h ago`;
    return `${Math.round(s / 86_400)}d ago`;
}

function resets(ms: number): string {
    const d = new Date(ms);
    const sameDay = d.toDateString() === new Date().toDateString();
    const time = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    return sameDay ? `today ${time}` : `${d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' })} ${time}`;
}

/** The freshest report per agent across machines. */
function merged(all: Record<number, PlanUsage[]>): PlanUsage[] {
    const best = new Map<string, PlanUsage>();
    for (const list of Object.values(all)) {
        for (const u of list) {
            const have = best.get(u.agent);
            if (!have || u.updatedAt > have.updatedAt) best.set(u.agent, u);
        }
    }
    return [...best.values()].sort((a, b) => ORDER.indexOf(agentId(a.agent)) - ORDER.indexOf(agentId(b.agent)));
}

export function UsageChip() {
    const live = useLive();
    const [open, setOpen] = useState(false);
    const root = useRef<HTMLDivElement>(null);

    useEffect(() => {
        if (!open) return;
        const onDown = (e: PointerEvent) => {
            if (!root.current?.contains(e.target as Node)) setOpen(false);
        };
        const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
        window.addEventListener('pointerdown', onDown);
        window.addEventListener('keydown', onKey);
        return () => {
            window.removeEventListener('pointerdown', onDown);
            window.removeEventListener('keydown', onKey);
        };
    }, [open]);

    const agents = merged(live.usage);
    if (agents.length === 0) return null;
    const measured = agents.filter((u) => u.windows.length > 0);
    const worst = Math.max(0, ...measured.flatMap((u) => u.windows.map((w) => w.usedPercentage)));

    return (
        <div className="usage" ref={root}>
            <button
                type="button"
                className={`hchip usage-chip ${level(worst)}`}
                aria-expanded={open}
                aria-haspopup="true"
                aria-label="Plan usage"
                title="Plan usage"
                onClick={() => setOpen((o) => !o)}
            >
                {measured.length === 0 && <span>Plans</span>}
                {measured.map((u) => (
                    <span
                        key={u.agent}
                        className="usage-chip-agent"
                        title={`${AGENTS[agentId(u.agent)].name}: ${u.windows.map((w) => `${label(w, u.agent)} ${Math.round(w.usedPercentage)}%`).join(', ')}`}
                    >
                        <AgentLogo agent={agentId(u.agent)} size={13} />
                        {u.windows.map((w, i) => (
                            <span key={w.id}>
                                {i > 0 && <span aria-hidden="true"> · </span>}
                                <strong className={level(w.usedPercentage)}>{Math.round(w.usedPercentage)}%</strong>
                            </span>
                        ))}
                    </span>
                ))}
            </button>
            {open && (
                <div className="usage-panel" role="group" aria-label="Plan usage">
                    <div className="usage-panel-head">
                        <strong>Plan usage</strong>
                        <span className="text-xs text-faint">Reported by Termlab</span>
                    </div>
                    {agents.map((u) => (
                        <AgentUsage key={u.agent} usage={u} />
                    ))}
                </div>
            )}
        </div>
    );
}

function AgentUsage({ usage: u }: { usage: PlanUsage }) {
    const id = agentId(u.agent);
    const stale = u.windows.length > 0 && Date.now() - u.updatedAt > STALE_MS;
    const state = u.connected === false ? 'Not signed in' : u.connected ? 'Connected' : null;

    return (
        <section className="usage-agent-group" aria-label={AGENTS[id].name}>
            <div className="usage-agent-head">
                <span className="usage-agent-name">
                    <AgentLogo agent={id} size={15} />
                    <strong>{AGENTS[id].name}</strong>
                    {u.plan && <span className="usage-plan">{u.plan}</span>}
                </span>
                <span className={u.connected === false ? 'usage-off' : 'text-faint'}>
                    {state}
                    {state && u.windows.length > 0 ? ' · ' : ''}
                    {u.windows.length > 0 ? ago(u.updatedAt) : ''}
                    {stale ? ' · may be out of date' : ''}
                </span>
            </div>
            {u.windows.map((w) => {
                const percent = Math.round(w.usedPercentage);
                const name = label(w, u.agent);
                return (
                    <div key={w.id} className="usage-window">
                        <div className="usage-window-head">
                            <span>{name}</span>
                            <span className="usage-percent">{percent}% used</span>
                        </div>
                        <div
                            className={`usage-bar ${level(w.usedPercentage)}`}
                            role="progressbar"
                            aria-label={name}
                            aria-valuemin={0}
                            aria-valuemax={100}
                            aria-valuenow={Math.min(100, percent)}
                        >
                            <span style={{ width: `${Math.min(100, Math.max(0, w.usedPercentage))}%` }} />
                        </div>
                        {w.resetsAt && <div className="text-xs text-faint">Resets {resets(w.resetsAt)}</div>}
                    </div>
                );
            })}
            {u.windows.length === 0 && u.note && <p className="m-0 text-xs text-dim">{u.note}</p>}
        </section>
    );
}
