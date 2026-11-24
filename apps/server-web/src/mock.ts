/**
 * A fake server for working on the UI without termlab-server (development
 * only; `?mock=1`). Never imported by production builds.
 */
import type { Machine, RemoteSession } from '@/types';

const now = () => new Date().toISOString();
const ago = (s: number) => new Date(Date.now() - s * 1000).toISOString();

const machines: Machine[] = [
    { id: 1, name: 'studio-mac', os: 'macOS 15 · Apple Silicon', version: '0.2.9', online: true, last_seen_at: now(), sessions: 4, running: 3, notes_sync: true },
    { id: 2, name: 'build-box', os: 'Ubuntu 24.04 · x86-64', version: '0.2.7', online: false, last_seen_at: ago(3600), sessions: 1, running: 0 },
];

function session(id: string, over: Partial<RemoteSession>): RemoteSession {
    return {
        id, machine_id: 1, machine: 'studio-mac', project: 'api-server', title: 'Session', agent: 'claude', model: 'Opus',
        branch: 'main', status: 'running', approval: null, preview: ['$ npm test', 'PASS test/auth.spec.ts'], tokens: 61000,
        context_pct: 28, changed_files: [], status_changed_at: ago(120), updated_at: now(), ...over,
    };
}

let sessions: RemoteSession[] = [
    session('a1', {
        title: 'Add invoices table', status: 'needs_you',
        approval: { kind: 'permission', question: 'Claude Code wants to run a command', detail: 'npm run db:migrate', options: [] },
        preview: ['● Created migrations/0042_invoices.sql', '● Updated src/models/invoice.ts (+71 −4)', '? Run command: npm run db:migrate'],
        changed_files: [{ path: 'migrations/0042_invoices.sql', added: 38, removed: 0 }, { path: 'src/models/invoice.ts', added: 71, removed: 4 }],
        context_pct: 62, tokens: 184000,
    }),
    session('a2', { title: 'Fix flaky auth tests', agent: 'codex', model: 'gpt-5-codex', branch: 'fix/auth-tests' }),
    session('a3', { project: 'web-app', title: 'Accessibility pass', status: 'done', preview: ['● Fixed 9 contrast issues', 'Done. All checks pass.'] }),
    session('a4', { project: 'docs', title: 'Build API reference', agent: 'antigravity', status: 'error', preview: ['Error: cannot resolve ./openapi.yaml', 'Process exited with code 1'] }),
    session('b1', { machine_id: 2, machine: 'build-box', project: 'infra', title: 'Upgrade Postgres', agent: 'vibe', status: 'idle' }),
];

let signedIn = true;
let setupRequired = false;

const json = (data: unknown, status = 200) =>
    new Response(status === 204 ? null : JSON.stringify(data), { status, headers: { 'Content-Type': 'application/json' } });

const QR = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21"><rect width="21" height="21" fill="#fff"/><path d="M1 1h7v7H1zM13 1h7v7h-7zM1 13h7v7H1z" fill="#000"/></svg>';

const PROJECT_NAMES = ['api-server', 'web-app', 'docs', 'brightness', 'democms'];
let mockLabels = [
    { id: 1, name: 'Bug', color: 'red' },
    { id: 2, name: 'Client work', color: 'blue' },
    { id: 3, name: 'From GitHub', color: 'purple' },
    { id: 4, name: 'Urgent', color: 'orange' },
];

/** An incoming webhook as the server answers it, from what the form sent. */
function mockHook(h: Record<string, unknown>) {
    const folderId = typeof h.folder_id === 'number' && h.folder_id > 0 ? h.folder_id : null;
    const project = folderId ? (PROJECT_NAMES[folderId - 1] ?? 'api-server') : null;

    return {
        id: h.id as number,
        name: (h.name as string) || 'Untitled',
        machine_id: 1,
        machine: 'studio-mac',
        folder_id: folderId,
        folder_path: project ? `/Users/me/Code/${project}` : null,
        project,
        project_missing: false,
        project_template: (h.project_template as string) ?? '',
        agent: (h.agent as string) ?? 'claude',
        permission: (h.permission as string) ?? 'ask_first',
        model: (h.model as string) || null,
        title: (h.title as string) ?? '',
        message: (h.message as string) ?? '{{body}}',
        label_ids: (h.label_ids as number[]) ?? [],
        labels_template: (h.labels_template as string) ?? '',
        create_labels: !!h.create_labels,
        notify: h.notify !== false,
        enabled: h.enabled !== false,
        last_call_at: (h.last_call_at as string | null) ?? null,
        url_hint: `${location.origin}/api/hooks/th_x7Kq…`,
    };
}

let mockHooks = [
    mockHook({ id: 2, name: 'GitHub issues, any repo', folder_id: 0, project_template: '{{body.repository.name}}', agent: 'claude', permission: 'auto_edit', title: '#{{body.issue.number}} {{body.issue.title}}', message: 'Fix this issue:\n\n{{body.issue.body}}', label_ids: [3], labels_template: '{{body.labels}}', last_call_at: new Date(Date.now() - 300_000).toISOString() }),
    mockHook({ id: 1, name: 'Phone shortcut', folder_id: 3, agent: 'claude', permission: 'ask_first', message: '{{body}}', label_ids: [4], last_call_at: new Date(Date.now() - 86_400_000).toISOString() }),
];

export function installMock(mode: string): void {
    signedIn = mode === '1';
    setupRequired = mode === 'setup';

    const realFetch = window.fetch.bind(window);
    window.fetch = async (input, init) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.pathname + input.search : input.url;
        if (!url.startsWith('/api/')) {
            return realFetch(input, init);
        }
        const method = init?.method ?? 'GET';
        const body: unknown = typeof init?.body === 'string' ? JSON.parse(init.body) : null;
        const path = url.split('?')[0];
        await new Promise((r) => setTimeout(r, 150));

        if (path === '/api/state') return json({ setup_required: setupRequired, signed_in: signedIn, version: '0.2.7', server_name: 'termlab.example.com', vapid_public_key: null });
        if (path === '/api/setup/start') return json({ totp_secret: 'JBSWY3DPEHPK3PXP', otpauth_url: 'otpauth://totp/Termlab?secret=JBSWY3DPEHPK3PXP', qr_svg: QR });
        if (path === '/api/setup/finish' || path === '/api/login') {
            setupRequired = false;
            signedIn = true;
            return json(null, 204);
        }
        if (path === '/api/logout') {
            signedIn = false;
            return json(null, 204);
        }
        if (path === '/api/overview') return json({
                machines,
                sessions: sessions.map((s, i) => ({ ...s, directory: `/Users/me/Code/${s.project}`, project_order: i, created_at: Date.now() - i * 60_000, on_hold: s.id === 'a3' })),
                notes: [{ machine_id: 1, path: '/Users/me/Code/api-server', text: '- Deploy with ./deploy.sh\n- Staging DB resets nightly', updated_at: Date.now() }],
                projects: [
                    { machine_id: 1, projects: [...new Set(sessions.filter((x) => x.machine_id === 1).map((x) => x.project)), 'brightness', 'democms'].map((name, i) => ({ id: i + 1, name, path: `/Users/me/Code/${name}`, order: i })), agents: [{ agent: 'claude', name: 'Claude Code', model_flag: true, models: ['opus', 'sonnet'] }, { agent: 'codex', name: 'Codex', model_flag: true, models: [] }, { agent: 'shell', name: 'Terminal', model_flag: false, models: [] }] },
                ],
                usage: [{ machine_id: 1, usage: [
                    { agent: 'claude', windows: [{ id: 'five_hour', usedPercentage: 68, resetsAt: Date.now() + 3_600_000 }, { id: 'seven_day', usedPercentage: 11, resetsAt: Date.now() + 86_400_000 * 3 }], updatedAt: Date.now() - 60_000, connected: true, plan: 'max' },
                    { agent: 'codex', windows: [{ id: 'seven_day', usedPercentage: 25, resetsAt: Date.now() + 86_400_000 * 5 }], updatedAt: Date.now() - 120_000, connected: true, plan: 'pro' },
                ] }],
            });
        if (path.startsWith('/api/sessions/')) {
            const id = path.split('/')[3];
            sessions = sessions.map((s) => (s.id === id ? { ...s, status: 'running', approval: null } : s));
            setTimeout(() => MockSocket.current?.emit({ type: 'session.updated', session: sessions.find((s) => s.id === id) }), 600);
            return json({ command_id: 'c1' }, 202);
        }
        if (path === '/api/pairing-codes') return json({ code: 'K7QX-2MPD', expires_at: new Date(Date.now() + 600_000).toISOString() });
        if (/^\/api\/machines\/\d+\/labels$/.test(path) && method === 'POST') {
            const { name, color } = body as { name: string; color: string };
            const label = mockLabels.find((l) => l.name.toLowerCase() === name.toLowerCase()) ?? { id: Math.max(0, ...mockLabels.map((l) => l.id)) + 1, name, color };
            if (!mockLabels.includes(label)) mockLabels = [...mockLabels, label];
            return json({ label, labels: mockLabels });
        }
        if (path.startsWith('/api/machines')) return json(method === 'DELETE' ? null : machines[0], method === 'DELETE' ? 204 : 200);
        if (path === '/api/webhooks' && method === 'GET')
            return json({
                endpoints: [{ id: 1, url: 'https://hooks.slack.com/services/T000/B000/XXXX', description: 'Ping #agents', events: ['session.needs_you', 'approval.requested'], enabled: true, health: 'healthy', last_delivery_at: ago(120), method: 'POST', headers: [{ name: 'X-Team', value: 'agents' }], auth: { type: 'bearer', name: null, set: true } }],
                events: [
                    { name: 'session.needs_you', description: 'A session waits for input' },
                    { name: 'approval.requested', description: 'An agent asks to run a command or edit a file' },
                    { name: 'session.done', description: 'A session finished its task' },
                    { name: 'session.error', description: 'A session exited with an error' },
                    { name: 'session.started', description: 'A new session was opened' },
                    { name: 'machine.offline', description: 'A paired machine stopped reporting' },
                ],
            });
        if (path === '/api/incoming' && method === 'GET') return json({ hooks: mockHooks, labels: { 1: mockLabels } });
        if (path === '/api/incoming' && method === 'POST') {
            const hook = mockHook({ ...(body as object), id: Math.max(0, ...mockHooks.map((h) => h.id)) + 1 });
            mockHooks = [hook, ...mockHooks];
            return json(hook, 201);
        }
        if (path.startsWith('/api/incoming/') && method === 'PATCH') {
            const id = Number(path.split('/')[3]);
            mockHooks = mockHooks.map((h) => (h.id === id ? mockHook({ ...h, ...(body as object) }) : h));
            return json(mockHooks.find((h) => h.id === id));
        }
        if (path.startsWith('/api/incoming/') && method === 'DELETE') {
            mockHooks = mockHooks.filter((h) => h.id !== Number(path.split('/')[3]));
            return json(null, 204);
        }
        if (path.startsWith('/api/incoming/') && (path.endsWith('/url') || path.endsWith('/roll'))) return json({ url: `${location.origin}/api/hooks/th_x7KqP2mVn8sLwR4tYb6cZ1dF0gHj` });
        if (path.endsWith('/calls'))
            return json([
                { id: 3, ip: '140.82.115.4', status: 'started', session_id: 'a1', error: null, created_at: ago(300) },
                { id: 2, ip: '140.82.115.4', status: 'offline', session_id: null, error: 'studio-mac is offline.', created_at: ago(7200) },
                { id: 1, ip: '86.88.1.2', status: 'failed', session_id: null, error: 'The project folder is gone.', created_at: ago(86400) },
            ]);
        if (path.endsWith('/deliveries'))
            return json([
                { id: 1, event: 'approval.requested', response_code: '200', ok: true, pending: false, attempt: 1, duration_ms: 120, created_at: ago(120), delivered_at: ago(119) },
                { id: 2, event: 'session.needs_you', response_code: '502', ok: false, pending: false, attempt: 3, duration_ms: 900, created_at: ago(3600), delivered_at: ago(3500) },
            ]);
        if (path.endsWith('/secret') || path.endsWith('/roll')) return json({ secret: 'whsec_mock_0123456789abcdef' });
        if (path === '/api/account/sessions')
            return json([
                { id: 1, user_agent: navigator.userAgent, ip: '127.0.0.1', created_at: ago(3600), last_seen_at: now(), current: true },
                { id: 2, user_agent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) Safari/604.1', ip: '86.88.1.2', created_at: ago(86400), last_seen_at: ago(7200), current: false },
            ]);
        if (path === '/api/account/totp/start') return json({ totp_secret: 'JBSWY3DPEHPK3PXP', otpauth_url: 'otpauth://totp/Termlab?secret=JBSWY3DPEHPK3PXP', qr_svg: QR });
        if (path === '/api/audit')
            return json({
                entries: [
                    { id: 3, action: 'approval.allow_once', detail: 'npm run db:migrate', machine: 'studio-mac', session_title: 'Add invoices table', ip: '86.88.1.2', created_at: ago(300) },
                    { id: 2, action: 'login', detail: null, machine: null, session_title: null, ip: '86.88.1.2', created_at: ago(3600) },
                ],
                next: null,
            });
        if (path === '/api/push' && method === 'GET') return json({ public_key: '', subscriptions: [] });
        return json(null, method === 'GET' ? 200 : 204);
    };

    // @ts-expect-error replacing the global for the mock
    window.WebSocket = MockSocket;
}

class MockSocket {
    static current: MockSocket | null = null;
    static OPEN = 1;
    readyState = 0;
    onopen: (() => void) | null = null;
    onmessage: ((e: { data: string }) => void) | null = null;
    onclose: (() => void) | null = null;

    constructor() {
        MockSocket.current = this;
        setTimeout(() => {
            this.readyState = 1;
            this.onopen?.();
        }, 100);
    }

    emit(msg: unknown) {
        this.onmessage?.({ data: JSON.stringify(msg) });
    }

    send(raw: string) {
        const msg = JSON.parse(raw) as { type: string; id: string; on?: boolean; data?: string };
        if (msg.type === 'watch' && msg.on) {
            const s = sessions.find((x) => x.id === msg.id);
            const text = (s?.preview ?? []).join('\r\n') + '\r\n' + '─'.repeat(160) + '\r\n\x1b[32m❯\x1b[0m ';
            setTimeout(() => {
                this.emit({ type: 'watch', id: msg.id, live: true });
                this.emit({ type: 'output', id: msg.id, data: btoa(unescape(encodeURIComponent(text))), snapshot: true, cols: 160, rows: 24 });
            }, 300);
        }
        if (msg.type === 'input' && msg.data) {
            this.emit({ type: 'output', id: msg.id, data: msg.data, snapshot: false, cols: null, rows: null });
        }
    }

    close() {
        this.readyState = 3;
    }
}
