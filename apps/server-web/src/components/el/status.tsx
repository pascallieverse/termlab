import { STATUS_LABEL } from '@/lib/format';
import { cn } from '@/lib/utils';
import type { SessionStatus } from '@/types';

const PILL: Record<SessionStatus, string> = {
    needs_you: 'bg-amber-bg text-amber-soft',
    running: 'bg-info-bg text-info',
    idle: 'bg-idle-bg text-dim',
    done: 'bg-ok-bg text-ok',
    error: 'bg-bad-bg text-bad',
};

const DOT: Record<SessionStatus, string> = {
    needs_you: 'bg-amber',
    running: 'bg-info-dot',
    idle: 'bg-faint',
    done: 'bg-ok-dot',
    error: 'bg-bad-dot',
};

export function StatusPill({
    status,
    className,
}: {
    status: SessionStatus;
    className?: string;
}) {
    return (
        <span
            className={cn(
                'rounded-full px-[9px] py-[3px] text-[11px] font-semibold whitespace-nowrap',
                PILL[status],
                className,
            )}
        >
            {STATUS_LABEL[status]}
        </span>
    );
}

export function StatusDot({
    status,
    className,
}: {
    status: SessionStatus;
    className?: string;
}) {
    return (
        <span
            aria-hidden="true"
            className={cn(
                'size-2 flex-none rounded-full',
                DOT[status],
                className,
            )}
        />
    );
}

export type Tone = 'ok' | 'bad' | 'off' | 'info' | 'amber';

const TONE: Record<Tone, string> = {
    ok: 'bg-ok-bg text-ok',
    bad: 'bg-bad-bg text-bad',
    off: 'bg-idle-bg text-dim',
    info: 'bg-info-bg text-info',
    amber: 'bg-amber-bg text-amber-soft',
};

export function Pill({
    tone,
    children,
    className,
}: {
    tone: Tone;
    children: React.ReactNode;
    className?: string;
}) {
    return (
        <span
            className={cn(
                'rounded-full px-2.5 py-1 text-xs font-semibold whitespace-nowrap',
                TONE[tone],
                className,
            )}
        >
            {children}
        </span>
    );
}
