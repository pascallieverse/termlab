/**
 * JSON calls to the server's own API (docs/server-protocol.md). Every request
 * other than GET carries `X-Termlab-Request: 1`, the server's CSRF guard.
 */
export class ApiError extends Error {
    constructor(
        message: string,
        public status: number,
    ) {
        super(message);
    }
}

/** Called when the server says the session is gone (401), so the app can show sign-in. */
let onUnauthorized: (() => void) | null = null;

export function setUnauthorizedHandler(handler: (() => void) | null): void {
    onUnauthorized = handler;
}

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (method !== 'GET') {
        headers['X-Termlab-Request'] = '1';
    }
    if (body !== undefined) {
        headers['Content-Type'] = 'application/json';
    }

    let response: Response;
    try {
        response = await fetch(url, {
            method,
            credentials: 'same-origin',
            headers,
            body: body === undefined ? undefined : JSON.stringify(body),
        });
    } catch {
        throw new ApiError('Could not reach the server.', 0);
    }

    const text = await response.text();
    let data: unknown = null;
    if (text) {
        try {
            data = JSON.parse(text);
        } catch {
            data = null;
        }
    }

    if (!response.ok) {
        const message =
            (data as { message?: string } | null)?.message ??
            (response.status === 429
                ? 'Too many attempts. Wait a few minutes and try again.'
                : `The server answered ${response.status}.`);
        if (response.status === 401 && !url.startsWith('/api/login') && !url.startsWith('/api/setup')) {
            onUnauthorized?.();
        }
        throw new ApiError(message, response.status);
    }

    return data as T;
}

export const api = {
    get: <T>(url: string) => request<T>('GET', url),
    post: <T = unknown>(url: string, body?: unknown) => request<T>('POST', url, body ?? {}),
    patch: <T = unknown>(url: string, body: unknown) => request<T>('PATCH', url, body),
    put: <T = unknown>(url: string, body: unknown) => request<T>('PUT', url, body),
    del: <T = unknown>(url: string) => request<T>('DELETE', url),
};

export function errorMessage(e: unknown, fallback: string): string {
    return e instanceof ApiError ? e.message : fallback;
}
