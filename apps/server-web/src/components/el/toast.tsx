import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

type Tone = 'success' | 'error' | 'info';
type Toast = { id: number; tone: Tone; message: string };
type Api = { success: (m: string) => void; error: (m: string) => void; info: (m: string) => void };

const ToastContext = createContext<Api | null>(null);

export function useToast(): Api {
    const t = useContext(ToastContext);
    if (!t) {
        throw new Error('useToast must be used inside <ToastProvider>');
    }

    return t;
}

let next = 1;

/** Short messages in the corner, announced to screen readers. */
export function ToastProvider({ children }: { children: ReactNode }) {
    const [toasts, setToasts] = useState<Toast[]>([]);

    const push = useCallback((tone: Tone, message: string) => {
        const id = next++;
        setToasts((list) => [...list.slice(-3), { id, tone, message }]);
        window.setTimeout(() => setToasts((list) => list.filter((t) => t.id !== id)), tone === 'error' ? 6000 : 4000);
    }, []);

    const api = useMemo<Api>(
        () => ({
            success: (m) => push('success', m),
            error: (m) => push('error', m),
            info: (m) => push('info', m),
        }),
        [push],
    );

    return (
        <ToastContext.Provider value={api}>
            {children}
            <div
                role="status"
                aria-live="polite"
                className="pointer-events-none fixed right-4 bottom-[max(1rem,env(safe-area-inset-bottom))] z-50 flex w-[min(360px,calc(100vw-2rem))] flex-col gap-2 max-md:bottom-20"
            >
                {toasts.map((t) => (
                    <div
                        key={t.id}
                        className={cn(
                            'pointer-events-auto rounded-[10px] border px-4 py-3 text-sm shadow-lg',
                            t.tone === 'error'
                                ? 'border-bad-line bg-bad-bg text-bad'
                                : t.tone === 'success'
                                  ? 'border-line-strong bg-panel text-ink'
                                  : 'border-line-strong bg-panel text-ink-soft',
                        )}
                    >
                        {t.message}
                    </div>
                ))}
            </div>
        </ToastContext.Provider>
    );
}
