import { cn } from '@/lib/utils';

export type SegmentOption<T extends string> = { value: T; label: string };

/** A row of toggle buttons, one pressed: the desktop app's `.segmented`. */
export function Segmented<T extends string>({
    options,
    value,
    onChange,
    label,
    size = 'md',
    stretch = false,
    dark = false,
    className,
}: {
    options: SegmentOption<T>[];
    value: T;
    onChange: (value: T) => void;
    label: string;
    size?: 'sm' | 'md';
    stretch?: boolean;
    dark?: boolean;
    className?: string;
}) {
    return (
        <div
            role="group"
            aria-label={label}
            className={cn('segmented', dark && 'dark', stretch && 'grid auto-cols-fr grid-flow-col', className)}
        >
            {options.map((option) => {
                const on = option.value === value;

                return (
                    <button
                        key={option.value}
                        type="button"
                        aria-pressed={on}
                        onClick={() => onChange(option.value)}
                        className={cn(on && 'on', size === 'sm' ? 'text-[13px]' : 'text-sm', stretch && 'min-h-9')}
                    >
                        {option.label}
                    </button>
                );
            })}
        </div>
    );
}
