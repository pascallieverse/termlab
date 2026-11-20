import { Link, useLocation } from 'react-router-dom';
import { cn } from '@/lib/utils';

export type NavItem = { to: string; label: string; icon: string };

/** The app's sections, in the sidebar and in the phone's bottom bar. */
export const NAV: NavItem[] = [
    { to: '/remote', label: 'Sessions', icon: 'M3 4h18v16H3zM7 10l3 2-3 2M12 15h4' },
    { to: '/machines', label: 'Machines', icon: 'M4 4h16v12H4zM8 20h8M12 16v4' },
    { to: '/webhooks', label: 'Webhooks', icon: 'M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10 21h4' },
    { to: '/settings', label: 'Settings', icon: 'M12 4a4 4 0 1 1 0 8 4 4 0 0 1 0-8zM4 21c1.5-4 4.5-6 8-6s6.5 2 8 6' },
];

export function NavIcon({ d, size = 22 }: { d: string; size?: number }) {
    return (
        <svg
            width={size}
            height={size}
            viewBox="0 0 24 24"
            aria-hidden="true"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
        >
            <path d={d} />
        </svg>
    );
}

export function useCurrentSection(): string {
    const path = useLocation().pathname;

    return NAV.find((n) => path.startsWith(n.to))?.to ?? '';
}

/** The phone's bottom bar. */
export function BottomNav() {
    const current = useCurrentSection();

    return (
        <nav
            aria-label="Sections"
            className="grid grid-cols-4 border-t border-line-soft bg-surface px-1 pt-1.5 pb-[max(22px,env(safe-area-inset-bottom))]"
        >
            {NAV.map((n) => {
                const on = current === n.to;

                return (
                    <Link
                        key={n.to}
                        to={n.to}
                        aria-current={on ? 'page' : undefined}
                        className={cn(
                            'flex min-h-11 flex-col items-center gap-0.5 py-1.5 text-[11px] no-underline',
                            on ? 'text-amber hover:text-amber' : 'text-dim hover:text-ink',
                        )}
                    >
                        <NavIcon d={n.icon} />
                        {n.label}
                    </Link>
                );
            })}
        </nav>
    );
}
