import type { ReactNode } from 'react';
import { Link, useLocation } from 'react-router-dom';
import { Logo } from '@/components/el/logo';
import { useLive } from '@/live/LiveProvider';
import { cn } from '@/lib/utils';

/** Line icons drawn like the desktop app's (24px grid, 2px stroke). */
export const ICONS = {
    sessions: <path d="M3 4h18v16H3zM7 10l3 2-3 2M12 15h4" />,
    settings: (
        <>
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
        </>
    ),
    machines: <path d="M4 4h16v12H4zM8 20h8M12 16v4" />,
    webhooks: (
        <>
            <path d="M18 16.98h-5.99c-1.1 0-1.95.94-2.48 1.9A4 4 0 0 1 2 17c.01-.7.2-1.4.57-2" />
            <path d="m6 17 3.13-5.78c.53-.97.1-2.18-.5-3.1a4 4 0 1 1 6.89-4.06" />
            <path d="m12 6 3.13 5.73C15.66 12.7 16.9 13 18 13a4 4 0 0 1 0 8" />
        </>
    ),
    bell: <path d="M6 16V11a6 6 0 1 1 12 0v5l1.5 2h-15zM10 20.5a2.2 2.2 0 0 0 4 0" />,
    palette: (
        <>
            <path d="M12 3a9 9 0 1 0 0 18c1.1 0 1.8-.8 1.8-1.8 0-.5-.2-.9-.5-1.2-.3-.3-.5-.7-.5-1.2 0-1 .8-1.8 1.8-1.8H17a4 4 0 0 0 4-4c0-4.4-4-8-9-8z" />
            <circle cx="7.5" cy="11.5" r="1" fill="currentColor" />
            <circle cx="10.5" cy="7.5" r="1" fill="currentColor" />
            <circle cx="15" cy="8" r="1" fill="currentColor" />
        </>
    ),
    account: (
        <>
            <path d="M12 3l7 3v5c0 4.5-3 8.3-7 10-4-1.7-7-5.5-7-10V6z" />
            <path d="M9 12l2 2 4-4" />
        </>
    ),
    audit: <path d="M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01" />,
    info: (
        <>
            <circle cx="12" cy="12" r="9" />
            <path d="M12 11v5M12 8h.01" />
        </>
    ),
    close: <path d="M6 6l12 12M18 6 6 18" />,
    chevron: <path d="m9 6 6 6-6 6" />,
    back: <path d="M15 18l-6-6 6-6" />,
    external: <path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" />,
} satisfies Record<string, ReactNode>;

export type IconName = keyof typeof ICONS;

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
    return (
        <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
            {ICONS[name]}
        </svg>
    );
}

/** Kept for pages that draw a nav icon from a path. */
export function NavIcon({ d, size = 22 }: { d: string; size?: number }) {
    return (
        <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d={d} />
        </svg>
    );
}

/** Sessions or Settings: the app's two places (Settings holds machines, webhooks and the rest). */
export function useCurrentSection(): '/remote' | '/settings' | '' {
    const path = useLocation().pathname;

    return path.startsWith('/remote') ? '/remote' : path.startsWith('/settings') ? '/settings' : '';
}

/** Whether the live connection to this server is up. */
export function ConnectionBadge() {
    const status = useLive().connection;
    const ok = status === 'connected';

    return (
        <span className="flex items-center gap-1.5 text-xs text-dim" role="status">
            <span aria-hidden="true" className="dot" style={{ background: ok ? '#3FB950' : '#F5A524' }} />
            {ok ? 'Connected' : status === 'connecting' ? 'Connecting…' : 'Reconnecting…'}
        </span>
    );
}

/**
 * The desktop app's header: the logo, then what the page puts there, then the connection
 * and the settings button. On phones the bottom bar takes over.
 */
export function TopBar({ children }: { children?: ReactNode }) {
    const current = useCurrentSection();
    const inSettings = current === '/settings';

    return (
        <header className="app-header">
            <Logo />
            {children}
            <div className="ml-auto flex items-center gap-2 max-md:hidden">
                <ConnectionBadge />
                <Link
                    to={inSettings ? '/remote' : '/settings'}
                    className={cn('icon-btn header-gear', inSettings && 'on')}
                    aria-label={inSettings ? 'Close settings' : 'Settings'}
                    title={inSettings ? 'Close settings' : 'Settings'}
                    aria-current={inSettings ? 'page' : undefined}
                >
                    <Icon name="settings" size={16} />
                </Link>
            </div>
        </header>
    );
}

/** The phone's bottom bar: Sessions and Settings. */
export function BottomNav() {
    const current = useCurrentSection();
    const live = useLive();
    const waiting = live.sessions.filter((s) => !s.on_hold && s.status === 'needs_you').length;
    const items = [
        { to: '/remote', label: 'Sessions', icon: 'sessions' as const },
        { to: '/settings', label: 'Settings', icon: 'settings' as const },
    ];

    return (
        <nav aria-label="Sections" className="grid grid-cols-2 border-t border-line bg-surface px-1 pt-1.5 pb-[max(22px,env(safe-area-inset-bottom))]">
            {items.map((n) => {
                const on = current === n.to;

                return (
                    <Link
                        key={n.to}
                        to={n.to}
                        aria-current={on ? 'page' : undefined}
                        className={cn(
                            'relative flex min-h-11 flex-col items-center gap-0.5 py-1.5 text-[11px] no-underline',
                            on ? 'text-amber hover:text-amber' : 'text-dim hover:text-ink',
                        )}
                    >
                        <Icon name={n.icon} size={22} />
                        {n.label}
                        {n.to === '/remote' && waiting > 0 && !on && (
                            <span className="folder-attention absolute top-0.5 left-[calc(50%+8px)]" aria-label={`${waiting} waiting on you`}>
                                {waiting}
                            </span>
                        )}
                    </Link>
                );
            })}
        </nav>
    );
}
