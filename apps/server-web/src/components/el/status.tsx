import { STATUS } from '@/lib/format';
import { cn } from '@/lib/utils';
import type { SessionStatus } from '@/types';

/** A coloured dot plus the label, outlined in the status colour (the desktop's StatusPill). */
export function StatusPill({ status, compact = false, className }: { status: SessionStatus; compact?: boolean; className?: string }) {
    const st = STATUS[status];
    const strong = status === 'needs_you' || status === 'error';

    return (
        <span
            className={cn('pill', compact && 'compact', className)}
            style={{ borderColor: st.color + '55', color: strong ? st.text : 'var(--color-ink-soft)' }}
        >
            <StatusDot status={status} />
            {st.label}
        </span>
    );
}

export function StatusDot({ status, className }: { status: SessionStatus; className?: string }) {
    return <span aria-hidden="true" className={cn('dot', className)} style={{ background: STATUS[status].color }} />;
}

/** The status in its text colour, as under a session's title in the sidebar. */
export function StatusText({ status }: { status: SessionStatus }) {
    return <span style={{ color: STATUS[status].text }}>{STATUS[status].label}</span>;
}

export type Tone = 'ok' | 'bad' | 'off' | 'info' | 'amber';

const TONE: Record<Tone, string> = {
    ok: '#3FB950',
    bad: '#F85149',
    off: '#8B929C',
    info: '#5B9BFF',
    amber: '#F5A524',
};

/** A pill for other states (machines, webhooks), drawn like the status pills. */
export function Pill({ tone, children, className }: { tone: Tone; children: React.ReactNode; className?: string }) {
    const strong = tone === 'bad' || tone === 'amber';

    return (
        <span
            className={cn('pill compact', className)}
            style={{
                borderColor: TONE[tone] + '55',
                color: strong ? (tone === 'bad' ? 'var(--color-bad)' : 'var(--color-amber-soft)') : 'var(--color-ink-soft)',
            }}
        >
            <span aria-hidden="true" className="dot" style={{ background: TONE[tone] }} />
            {children}
        </span>
    );
}
