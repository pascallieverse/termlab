import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** The bordered card used across the pages (the desktop's settings card). */
export function Panel({
    title,
    titleId,
    actions,
    children,
    className,
    bodyClassName,
    as: Tag = 'section',
}: {
    title?: ReactNode;
    titleId?: string;
    actions?: ReactNode;
    children: ReactNode;
    className?: string;
    bodyClassName?: string;
    as?: 'section' | 'div';
}) {
    return (
        <Tag
            aria-labelledby={title && titleId ? titleId : undefined}
            className={cn(
                'overflow-hidden rounded-[10px] border border-line bg-surface',
                className,
            )}
        >
            {title && (
                <div className="flex items-center justify-between gap-3 border-b border-line px-4 py-3">
                    <h2 id={titleId} className="m-0 text-sm font-semibold">
                        {title}
                    </h2>
                    {actions}
                </div>
            )}
            <div className={bodyClassName}>{children}</div>
        </Tag>
    );
}

export function StatCard({
    label,
    value,
    footer,
    highlight = false,
}: {
    label: string;
    value: ReactNode;
    footer?: ReactNode;
    highlight?: boolean;
}) {
    return (
        <div
            className={cn(
                'flex flex-col gap-1 rounded-[10px] border p-4',
                highlight
                    ? 'border-amber-line bg-amber-panel'
                    : 'border-line bg-surface',
            )}
        >
            <span
                className={cn(
                    'text-xs',
                    highlight ? 'text-amber-soft' : 'text-dim',
                )}
            >
                {label}
            </span>
            <span className="text-xl font-semibold">{value}</span>
            {footer && <span className="text-xs text-dim">{footer}</span>}
        </div>
    );
}
