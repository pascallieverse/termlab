import { useSyncExternalStore } from 'react';

const QUERY = '(max-width: 767px)';

function subscribe(callback: () => void) {
    const mql = window.matchMedia(QUERY);
    mql.addEventListener('change', callback);

    return () => mql.removeEventListener('change', callback);
}

/** True below Tailwind's md breakpoint: the phone layout of Termlab Remote. */
export function useNarrow(): boolean {
    return useSyncExternalStore(
        subscribe,
        () => window.matchMedia(QUERY).matches,
        () => false,
    );
}
