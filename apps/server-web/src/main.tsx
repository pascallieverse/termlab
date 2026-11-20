import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './app.css';

async function start() {
    // Development only: `?mock=1` (signed in), `?mock=login` or `?mock=setup` fakes the server.
    if (import.meta.env.DEV && new URLSearchParams(window.location.search).has('mock')) {
        const { installMock } = await import('./mock');
        installMock(new URLSearchParams(window.location.search).get('mock') ?? '1');
    }

    createRoot(document.getElementById('root')!).render(
        <StrictMode>
            <App />
        </StrictMode>,
    );
}

void start();

// The service worker shows push notifications; it needs https:// (or localhost).
if ('serviceWorker' in navigator && window.isSecureContext) {
    void navigator.serviceWorker.register('/sw.js', { scope: '/' }).catch(() => {});
}
