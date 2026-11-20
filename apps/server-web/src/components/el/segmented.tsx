import { cn } from '@/lib/utils';

export type SegmentOption<T extends string> = { value: T; label: string };

/**
 * A row of toggle buttons (the design's pill switchers), one pressed.
 */
export function Segmented<T extends string>({
    options,
    value,
    onChange,
    label,
    size = 'md',
    stretch = false,
    className,
}: {
    options: SegmentOption<T>[];
    value: T;
    onChange: (value: T) => void;
    label: string;
    size?: 'sm' | 'md';
    stretch?: boolean;
    className?: string;
}) {
    return (
        <div
            role="group"
            aria-label={label}
            className={cn(
                'gap-1 rounded-xl border border-line bg-panel p-1',
                stretch ? 'grid auto-cols-fr grid-flow-col' : 'flex',
                className,
            )}
        >
            {options.map((option) => {
                const on = option.value === value;

                return (
                    <button
                        key={option.value}
                        type="button"
                        aria-pressed={on}
                        onClick={() => onChange(option.value)}
                        className={cn(
                            'cursor-pointer rounded-[9px] font-medium whitespace-nowrap transition-colors',
                            size === 'md'
                                ? 'min-h-11 px-[18px] text-[15px]'
                                : 'min-h-[34px] px-3 text-[13px]',
                            on
                                ? 'bg-ink text-bg'
                                : 'bg-transparent text-dim hover:text-ink',
                        )}
                    >
                        {option.label}
                    </button>
                );
            })}
        </div>
    );
}
