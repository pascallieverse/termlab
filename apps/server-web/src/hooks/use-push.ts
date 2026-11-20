import { useCallback, useEffect, useState } from 'react';
import { useAppState } from '@/app-state';
import { api } from '@/lib/api';

export type PushState =
    | 'loading'
    | 'unsupported'
    | 'insecure' // push needs https:// (or localhost)
    | 'install' // iPhone/iPad: add to Home Screen first
    | 'denied'
    | 'off'
    | 'on';

function supported(): boolean {
    return 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window;
}

function needsInstall(): boolean {
    const ios = /iPad|iPhone|iPod/.test(navigator.userAgent);
    const standalone =
        window.matchMedia('(display-mode: standalone)').matches ||
        (navigator as Navigator & { standalone?: boolean }).standalone === true;

    return ios && !standalone;
}

function keyBytes(base64: string): Uint8Array<ArrayBuffer> {
    const padded = (base64 + '='.repeat((4 - (base64.length % 4)) % 4)).replace(/-/g, '+').replace(/_/g, '/');
    const raw = atob(padded);
    const bytes = new Uint8Array(new ArrayBuffer(raw.length));
    for (let i = 0; i < raw.length; i++) {
        bytes[i] = raw.charCodeAt(i);
    }

    return bytes;
}

function deviceLabel(): string {
    const ua = navigator.userAgent;
    const os = /iPhone/.test(ua)
        ? 'iPhone'
        : /iPad/.test(ua)
          ? 'iPad'
          : /Android/.test(ua)
            ? 'Android'
            : /Mac OS X/.test(ua)
              ? 'Mac'
              : /Windows/.test(ua)
                ? 'Windows'
                : /Linux/.test(ua)
                  ? 'Linux'
                  : 'Device';
    const browser = /Edg\//.test(ua)
        ? 'Edge'
        : /Firefox\//.test(ua)
          ? 'Firefox'
          : /Chrome\//.test(ua)
            ? 'Chrome'
            : /Safari\//.test(ua)
              ? 'Safari'
              : 'Browser';

    return `${browser} on ${os}`;
}

async function registration(): Promise<ServiceWorkerRegistration> {
    const existing = await navigator.serviceWorker.getRegistration('/');

    return existing ?? navigator.serviceWorker.register('/sw.js', { scope: '/' });
}

/** Push notifications on this browser or phone: whether they're on, and turning them on or off. */
export function usePush() {
    const key = useAppState().state.vapid_public_key;
    const [state, setState] = useState<PushState>('loading');
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        if (!window.isSecureContext) {
            setState('insecure');

            return;
        }
        if (!supported() || !key) {
            setState(needsInstall() ? 'install' : 'unsupported');

            return;
        }
        if (Notification.permission === 'denied') {
            setState('denied');

            return;
        }
        void navigator.serviceWorker.getRegistration('/').then(async (reg) => {
            const sub = reg ? await reg.pushManager.getSubscription() : null;
            setState(sub ? 'on' : 'off');
        });
    }, [key]);

    const enable = useCallback(async () => {
        if (!key) {
            return;
        }
        setError(null);
        setState('loading');
        try {
            const permission = await Notification.requestPermission();
            if (permission !== 'granted') {
                setState(permission === 'denied' ? 'denied' : 'off');

                return;
            }
            const reg = await registration();
            await navigator.serviceWorker.ready;
            const sub =
                (await reg.pushManager.getSubscription()) ??
                (await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) }));
            const json = sub.toJSON();
            await api.post<{ id: number }>('/api/push', {
                endpoint: json.endpoint,
                keys: json.keys,
                label: deviceLabel(),
            });
            setState('on');
        } catch (e) {
            setError(e instanceof Error ? e.message : 'Could not turn on notifications.');
            setState('off');
        }
    }, [key]);

    const disable = useCallback(async () => {
        setError(null);
        setState('loading');
        try {
            const reg = await navigator.serviceWorker.getRegistration('/');
            const sub = reg ? await reg.pushManager.getSubscription() : null;
            if (sub) {
                // The server lists each device's endpoint: remove the one that is this browser.
                const list = await api
                    .get<{ subscriptions: { id: number; endpoint: string }[] }>('/api/push')
                    .catch(() => ({ subscriptions: [] }));
                const mine = list.subscriptions.find((d) => d.endpoint === sub.endpoint);
                if (mine) {
                    await api.del(`/api/push/${mine.id}`).catch(() => {});
                }
                await sub.unsubscribe();
            }
            setState('off');
        } catch (e) {
            setError(e instanceof Error ? e.message : 'Could not turn off notifications.');
            setState('on');
        }
    }, []);

    return { state, error, enable, disable };
}
