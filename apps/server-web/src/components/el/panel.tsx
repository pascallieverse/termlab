import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** The bordered card used across the account pages. */
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
                'overflow-hidden rounded-[14px] border border-line bg-panel',
                className,
            )}
        >
            {title && (
                <div className="flex items-center justify-between gap-3 border-b border-line px-5 py-[18px]">
                    <h2 id={titleId} className="m-0 text-[17px] font-semibold">
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
                'flex flex-col gap-1.5 rounded-[14px] border p-5',
                highlight
                    ? 'border-amber-line bg-amber-panel'
                    : 'border-line bg-panel',
            )}
        >
            <span
                className={cn(
                    'text-[13px]',
                    highlight ? 'text-amber-soft' : 'text-dim',
                )}
            >
                {label}
            </span>
            <span className="font-display text-2xl font-semibold">{value}</span>
            {footer && <span className="text-[13px] text-dim">{footer}</span>}
        </div>
    );
}
