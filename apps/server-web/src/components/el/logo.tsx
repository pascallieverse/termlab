import { useId } from 'react';
import { Link } from 'react-router-dom';
import { cn } from '@/lib/utils';

/** The Termlab mark: a lab flask holding a terminal prompt (the desktop app's Logo). */
export function LogoMark({ size = 22 }: { size?: number }) {
    const mask = useId();

    return (
        <svg
            width={size}
            height={size}
            viewBox="0 0 24 24"
            fill="var(--color-amber)"
            stroke="var(--color-amber)"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
            className="shrink-0"
        >
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

/** "Termlab" with the mark, as in the desktop app's header, plus a quiet "Server". */
export function Logo({ to = '/remote', className }: { to?: string; className?: string }) {
    return (
        <Link to={to} className={cn('brand', className)}>
            <LogoMark />
            <span>
                Termlab <span className="brand-sub">Server</span>
            </span>
        </Link>
    );
}
