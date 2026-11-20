/*
 * Termlab Server service worker: shows push notifications for sessions
 * that need you, and opens the session when one is tapped.
 * The server sends JSON { title, body, url }.
 */
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

self.addEventListener('push', (event) => {
    let payload = {};
    try {
        payload = event.data ? event.data.json() : {};
    } catch {
        payload = { title: 'Termlab', body: event.data ? event.data.text() : '' };
    }

    const title = payload.title || 'Termlab';
    const url = payload.url || (payload.data && payload.data.url) || '/remote';
    event.waitUntil(
        self.registration.showNotification(title, {
            body: payload.body || '',
            icon: '/icon-192.png',
            badge: '/icon-192.png',
            tag: payload.tag || url,
            renotify: true,
            data: { url },
        }),
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    const url = new URL((event.notification.data && event.notification.data.url) || '/remote', self.location.origin).href;

    event.waitUntil(
        self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((windows) => {
            for (const client of windows) {
                if (new URL(client.url).origin === self.location.origin && 'focus' in client) {
                    return client.navigate(url).then((c) => (c || client).focus());
                }
            }

            return self.clients.openWindow(url);
        }),
    );
});
