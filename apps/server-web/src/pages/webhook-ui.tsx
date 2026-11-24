import { useEffect, useRef, type ReactNode } from 'react';
import { Button } from '@/components/el/button';
import { useToast } from '@/components/el/toast';
import { cn } from '@/lib/utils';

// Pieces both webhook tabs use.

export const inputClass = 'min-h-11 rounded-[9px] border border-line-strong bg-bg px-3 text-ink';
/** WebKit sizes a select by `height`, not `min-height`. */
export const selectClass = `${inputClass} h-11`;

export function Field({
    id,
    label,
    optional,
    hint,
    className,
    children,
}: {
    id: string;
    label: ReactNode;
    optional?: boolean;
    hint?: ReactNode;
    className?: string;
    children: ReactNode;
}) {
    return (
        <div className={cn('flex min-w-0 flex-col gap-1.5', className)}>
            <label htmlFor={id} className="text-sm font-medium">
                {label} {optional && <span className="font-normal text-dim">(optional)</span>}
            </label>
            {children}
            {hint && <span className="text-[13px] text-dim">{hint}</span>}
        </div>
    );
}

/** A secret shown on request (a signing secret, a hook's URL), with Copy. */
export function SecretValue({
    masked,
    load,
    busy,
    what,
    value,
    onValue,
}: {
    masked: string;
    load: () => void;
    busy: boolean;
    /** For the toasts: "Secret", "URL". */
    what: string;
    value: string | null;
    onValue: (v: string | null) => void;
}) {
    const toast = useToast();

    async function copy() {
        if (!value) return;
        try {
            await navigator.clipboard.writeText(value);
            toast.success(`${what} copied.`);
        } catch {
            toast.error(`Could not copy. Select the ${what.toLowerCase()} and copy it.`);
        }
    }

    return (
        <div className="flex flex-wrap gap-2">
            <code className="min-w-0 flex-[1_1_220px] overflow-x-auto rounded-lg border border-line bg-bg p-3 font-mono text-[13px] [overflow-wrap:anywhere] text-ink-soft">
                {value ?? masked}
            </code>
            <Button variant="outline" disabled={busy} onClick={() => (value ? onValue(null) : load())}>
                {value ? 'Hide' : 'Reveal'}
            </Button>
            {value && (
                <Button variant="outline" onClick={() => void copy()}>
                    Copy
                </Button>
            )}
        </div>
    );
}

/** A form opened from further down (Edit) scrolls itself into view. */
export function useScrollIntoView<T extends HTMLElement>() {
    const ref = useRef<T>(null);
    useEffect(() => {
        ref.current?.scrollIntoView({ block: 'start' });
    }, []);

    return ref;
}

/** Termlab's label colours (apps/desktop/src/sessionLabels.ts). */
const LABEL_COLORS: Record<string, string> = {
    red: '#f2716b',
    orange: '#f0a45d',
    yellow: '#e8c065',
    green: '#7fd17a',
    cyan: '#5fcfd6',
    blue: '#6aa8ff',
    purple: '#b58cf5',
    pink: '#f28cc0',
    gray: '#a3aab5',
};

export function labelHex(color: string): string {
    return LABEL_COLORS[color] ?? LABEL_COLORS.gray;
}

/** A label as Termlab draws it: a dot in its colour and its name. */
export function LabelChip({ name, color, className }: { name: string; color: string; className?: string }) {
    return (
        <span
            className={cn('inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs', className)}
            style={{ borderColor: labelHex(color) + '66', color: 'var(--color-ink-soft)' }}
        >
            <span aria-hidden="true" className="size-2 rounded-full" style={{ background: labelHex(color) }} />
            {name}
        </span>
    );
}
