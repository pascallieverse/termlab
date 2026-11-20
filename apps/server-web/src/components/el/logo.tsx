import { Link } from 'react-router-dom';
import { cn } from '@/lib/utils';

export function LogoMark({ size = 28 }: { size?: number }) {
    return (
        <svg
            width={size}
            height={size}
            viewBox="0 0 28 28"
            aria-hidden="true"
            className="shrink-0"
        >
            <rect
                x="1"
                y="1"
                width="26"
                height="26"
                rx="7"
                fill="none"
                stroke="#FFB547"
                strokeWidth="2"
            />
            <path
                d="M8 10l4 4-4 4M14 18h6"
                fill="none"
                stroke="#FFB547"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
            />
        </svg>
    );
}

export function Logo({
    to = '/remote',
    size = 28,
    className,
}: {
    to?: string;
    size?: number;
    className?: string;
}) {
    return (
        <Link
            to={to}
            className={cn(
                'flex items-center gap-2.5 text-ink no-underline hover:text-ink',
                className,
            )}
        >
            <LogoMark size={size} />
            <span className="font-display text-[19px] font-bold tracking-[-0.01em]">
                Termlab <span className="text-amber">Server</span>
            </span>
        </Link>
    );
}

export function TermlabIcon({
    className,
    alt = '',
}: {
    className?: string;
    alt?: string;
}) {
    return (
        <img
            src="/termlab-icon.png"
            alt={alt}
            className={cn('rounded-[12px]', className)}
        />
    );
}
