import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { LogoMark } from '@/components/el/logo';
import { useLive } from '@/live/LiveProvider';
import { cn } from '@/lib/utils';
import { BottomNav, NAV, NavIcon, useCurrentSection } from './nav';

/**
 * Machines, Webhooks and Settings: a sidebar on wide screens, a top bar and
 * the bottom bar on phones. (Sessions has its own full-screen layout.)
 */
export function AppShell({ children }: { children: ReactNode }) {
    const current = useCurrentSection();
    const live = useLive();
    const waiting = live.sessions.filter((s) => s.status === 'needs_you').length;

    return (
        <div className="flex min-h-dvh bg-bg text-[15px] leading-normal text-ink max-md:flex-col">
            <nav
                aria-label="Sections"
                className="flex w-[240px] flex-none flex-col gap-6 border-r border-line-soft bg-surface px-4 py-6 max-md:hidden"
            >
                <Link to="/remote" className="flex items-center gap-2.5 px-2 text-ink no-underline hover:text-ink">
                    <LogoMark size={26} />
                    <span className="font-display text-lg font-bold">
                        Termlab <span className="text-amber">Server</span>
                    </span>
                </Link>
                <div className="flex flex-col gap-0.5">
                    {NAV.map((n) => {
                        const on = current === n.to;

                        return (
                            <Link
                                key={n.to}
                                to={n.to}
                                aria-current={on ? 'page' : undefined}
                                className={cn(
                                    'flex items-center gap-3 rounded-lg px-3 py-2.5 no-underline',
                                    on ? 'bg-raised font-medium text-ink hover:text-ink' : 'text-dim hover:bg-raised/60 hover:text-ink',
                                )}
                            >
                                <NavIcon d={n.icon} size={18} />
                                <span className="flex-1">{n.label}</span>
                                {n.to === '/remote' && waiting > 0 && (
                                    <span className="min-w-[20px] rounded-full bg-amber px-1.5 text-center text-[11px] font-bold text-amber-ink">
                                        {waiting}
                                    </span>
                                )}
                            </Link>
                        );
                    })}
                </div>
                <p className="mt-auto mb-0 px-3 font-mono text-[11px] text-faint">
                    {live.connection === 'connected' ? 'live' : live.connection}
                </p>
            </nav>

            <header className="flex items-center gap-2.5 border-b border-line-soft bg-surface px-4 py-3 md:hidden">
                <LogoMark size={24} />
                <span className="font-display text-base font-semibold">
                    Termlab <span className="text-amber">Server</span>
                </span>
            </header>

            <main className="flex min-w-0 flex-1 flex-col gap-7 px-[clamp(16px,3vw,40px)] py-8 max-md:pb-6">{children}</main>

            <div className="sticky bottom-0 md:hidden">
                <BottomNav />
            </div>
        </div>
    );
}

export function PageHeader({
    title,
    description,
    actions,
}: {
    title: string;
    description?: ReactNode;
    actions?: ReactNode;
}) {
    return (
        <div className="flex flex-wrap items-center justify-between gap-4">
            <div>
                <h1 className="m-0 font-display text-[30px] tracking-[-0.02em]">{title}</h1>
                {description && <p className="mt-1 mb-0 text-dim">{description}</p>}
            </div>
            {actions && <div className="flex flex-wrap gap-2">{actions}</div>}
        </div>
    );
}
