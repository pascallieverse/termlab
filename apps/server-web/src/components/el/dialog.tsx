import { useEffect, useId, useRef, type ReactNode } from 'react';

/**
 * A modal dialog: traps nothing fancy, but closes on Escape and on the backdrop,
 * moves focus into the dialog and back to where it was when it closes.
 */
export function Dialog({
    open,
    onClose,
    title,
    description,
    children,
}: {
    open: boolean;
    onClose: () => void;
    title: ReactNode;
    description?: ReactNode;
    children: ReactNode;
}) {
    const box = useRef<HTMLDivElement>(null);
    const titleId = useId();
    const descId = useId();

    useEffect(() => {
        if (!open) {
            return;
        }
        const before = document.activeElement as HTMLElement | null;
        const first = box.current?.querySelector<HTMLElement>(
            'input, button, [href], select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        (first ?? box.current)?.focus();
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                onClose();
            }
        };
        document.addEventListener('keydown', onKey);

        return () => {
            document.removeEventListener('keydown', onKey);
            before?.focus?.();
        };
    }, [open, onClose]);

    if (!open) {
        return null;
    }

    return (
        <div
            className="fixed inset-0 z-40 flex items-end justify-center bg-[rgba(4,5,6,0.66)] p-4 sm:items-start sm:pt-[10vh]"
            onMouseDown={(e) => {
                if (e.target === e.currentTarget) {
                    onClose();
                }
            }}
        >
            <div
                ref={box}
                role="dialog"
                aria-modal="true"
                aria-labelledby={titleId}
                aria-describedby={description ? descId : undefined}
                tabIndex={-1}
                className="flex max-h-[88dvh] w-full max-w-md flex-col overflow-hidden rounded-[14px] border border-line-strong bg-panel text-ink shadow-[0_24px_80px_rgba(0,0,0,0.5)] outline-none"
            >
                <div className="flex items-start justify-between gap-3 border-b border-line px-[22px] py-[18px]">
                    <div className="flex flex-col gap-0.5">
                        <h2 id={titleId} className="m-0 text-lg font-semibold">
                            {title}
                        </h2>
                        {description && (
                            <p id={descId} className="m-0 text-[13px] text-dim">
                                {description}
                            </p>
                        )}
                    </div>
                    <button
                        type="button"
                        aria-label="Close"
                        onClick={onClose}
                        className="icon-btn"
                    >
                        <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                            <path d="M6 6l12 12M18 6L6 18" />
                        </svg>
                    </button>
                </div>
                <div className="flex flex-col gap-4 overflow-y-auto px-[22px] py-[18px]">{children}</div>
            </div>
        </div>
    );
}
