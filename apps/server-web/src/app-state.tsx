import { createContext, useContext } from 'react';
import type { ServerState } from '@/types';

type AppState = {
    state: ServerState;
    /** Asks the server again (after sign-in, sign-out or setup). */
    refresh: () => Promise<void>;
};

export const AppStateContext = createContext<AppState | null>(null);

export function useAppState(): AppState {
    const v = useContext(AppStateContext);
    if (!v) {
        throw new Error('useAppState must be used inside the app');
    }

    return v;
}
