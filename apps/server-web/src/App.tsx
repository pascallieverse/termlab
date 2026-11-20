import { useCallback, useEffect, useMemo, useState } from 'react';
import { BrowserRouter, Navigate, Route, Routes, useLocation } from 'react-router-dom';
import { AppStateContext } from '@/app-state';
import { ToastProvider } from '@/components/el/toast';
import { LiveProvider } from '@/live/LiveProvider';
import { api, setUnauthorizedHandler } from '@/lib/api';
import Login from '@/pages/Login';
import Remote from '@/pages/Remote';
import Settings from '@/pages/Settings';
import Setup from '@/pages/Setup';
import type { ServerState } from '@/types';

/**
 * The sessions, always; Settings opens over them as a dialog, as in the desktop app, so the
 * terminal you were watching keeps streaming behind it.
 */
function Main() {
    const path = useLocation().pathname;
    const settings = path === '/settings' || path.startsWith('/settings/');
    if (!settings && path !== '/remote') {
        return <Navigate to="/remote" replace />;
    }

    return (
        <>
            <Remote />
            {settings && <Settings />}
        </>
    );
}

export default function App() {
    const [state, setState] = useState<ServerState | null>(null);
    const [failed, setFailed] = useState(false);

    const refresh = useCallback(async () => {
        try {
            setState(await api.get<ServerState>('/api/state'));
            setFailed(false);
        } catch {
            setFailed(true);
        }
    }, []);

    useEffect(() => {
        void refresh();
    }, [refresh]);

    // Any 401 from the API means the session ended: show sign-in.
    useEffect(() => {
        setUnauthorizedHandler(() => setState((s) => (s ? { ...s, signed_in: false } : s)));

        return () => setUnauthorizedHandler(null);
    }, []);

    const signedOut = useCallback(() => setState((s) => (s ? { ...s, signed_in: false } : s)), []);
    const context = useMemo(() => (state ? { state, refresh } : null), [state, refresh]);

    if (!state || !context) {
        return (
            <div className="grid min-h-dvh place-items-center bg-bg p-6 text-center text-dim">
                {failed ? (
                    <div className="flex flex-col items-center gap-3">
                        <p className="m-0">Can’t reach the Termlab server.</p>
                        <button
                            type="button"
                            onClick={() => void refresh()}
                            className="min-h-11 cursor-pointer rounded-lg border border-line-strong px-4 text-ink hover:bg-raised"
                        >
                            Try again
                        </button>
                    </div>
                ) : (
                    <p className="m-0">Loading…</p>
                )}
            </div>
        );
    }

    return (
        <AppStateContext.Provider value={context}>
            <ToastProvider>
                <BrowserRouter>
                    {state.setup_required ? (
                        <Setup />
                    ) : !state.signed_in ? (
                        <Login />
                    ) : (
                        <LiveProvider onSignedOut={signedOut}>
                            <Routes>
                                {/* Before Settings held them. */}
                                <Route path="/machines" element={<Navigate to="/settings/machines" replace />} />
                                <Route path="/webhooks" element={<Navigate to="/settings/webhooks" replace />} />
                                <Route path="*" element={<Main />} />
                            </Routes>
                        </LiveProvider>
                    )}
                </BrowserRouter>
            </ToastProvider>
        </AppStateContext.Provider>
    );
}
