import {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
} from 'react';
import { api, errorMessage } from '@/lib/api';
import { useToast } from '@/components/el/toast';
import type { ApprovalChoice, Machine, RemoteSession } from '@/types';

export type Connection = 'connecting' | 'connected' | 'reconnecting';

/** What a terminal view receives for its session. */
export type TerminalEvent =
    | { type: 'output'; data: string; snapshot: boolean; cols: number | null; rows: number | null }
    | { type: 'watch'; live: boolean };

type TerminalListener = (event: TerminalEvent) => void;

type ServerMessage =
    | { type: 'session.updated'; session: RemoteSession }
    | { type: 'session.removed'; id: string }
    | { type: 'machine.updated'; machine: Machine }
    | { type: 'output'; id: string; data: string; snapshot: boolean; cols: number | null; rows: number | null }
    | { type: 'watch'; id: string; live: boolean };

export type SessionAction = 'answer' | 'message' | 'stop' | 'hold';

type Live = {
    connection: Connection;
    loaded: boolean;
    sessions: RemoteSession[];
    machines: Machine[];
    /** Actions sent but not yet confirmed by the machine, by session id. */
    pending: Record<string, string>;
    reload: () => Promise<void>;
    act: (session: RemoteSession, action: SessionAction, body?: Record<string, unknown>, label?: string) => Promise<boolean>;
    answer: (session: RemoteSession, choice: ApprovalChoice) => Promise<boolean>;
    /** Opens a terminal view: the machine streams it while at least one view is open. */
    watch: (id: string, listener: TerminalListener) => () => void;
    input: (id: string, data: string) => void;
};

const LiveContext = createContext<Live | null>(null);

export function useLive(): Live {
    const live = useContext(LiveContext);
    if (!live) {
        throw new Error('useLive must be used inside <LiveProvider>');
    }

    return live;
}

function wsUrl(): string {
    const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';

    return `${scheme}://${window.location.host}/api/ws`;
}

/**
 * The one live connection to the server (`/api/ws`): keeps sessions and
 * machines in sync and carries terminal output and keystrokes.
 */
export function LiveProvider({ children, onSignedOut }: { children: ReactNode; onSignedOut: () => void }) {
    const toast = useToast();
    const [connection, setConnection] = useState<Connection>('connecting');
    const [loaded, setLoaded] = useState(false);
    const [sessions, setSessions] = useState<RemoteSession[]>([]);
    const [machines, setMachines] = useState<Machine[]>([]);
    const [pending, setPending] = useState<Record<string, string>>({});

    const socket = useRef<WebSocket | null>(null);
    const listeners = useRef(new Map<string, Set<TerminalListener>>());

    const send = useCallback((message: unknown) => {
        const ws = socket.current;
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify(message));
        }
    }, []);

    const reload = useCallback(async () => {
        try {
            const data = await api.get<{ machines: Machine[]; sessions: RemoteSession[] }>('/api/overview');
            setMachines(data.machines);
            setSessions(data.sessions);
            setLoaded(true);
        } catch {
            // 401 is handled by the API client; anything else retries on reconnect.
        }
    }, []);

    useEffect(() => {
        let closed = false;
        let attempt = 0;
        let timer: number | undefined;

        const connect = () => {
            setConnection(attempt === 0 ? 'connecting' : 'reconnecting');
            const ws = new WebSocket(wsUrl());
            socket.current = ws;

            ws.onopen = () => {
                attempt = 0;
                setConnection('connected');
                // Catch up on anything missed while disconnected, and reopen terminal views.
                void reload();
                for (const id of listeners.current.keys()) {
                    ws.send(JSON.stringify({ type: 'watch', id, on: true }));
                }
            };

            ws.onmessage = (event) => {
                let msg: ServerMessage;
                try {
                    msg = JSON.parse(String(event.data)) as ServerMessage;
                } catch {
                    return;
                }

                switch (msg.type) {
                    case 'session.updated': {
                        const next = msg.session;
                        setSessions((list) =>
                            list.some((s) => s.id === next.id)
                                ? list.map((s) => (s.id === next.id ? next : s))
                                : [...list, next],
                        );
                        // A new state from the machine settles any action in flight.
                        setPending((p) => {
                            if (!(next.id in p)) {
                                return p;
                            }
                            const rest = { ...p };
                            delete rest[next.id];

                            return rest;
                        });
                        break;
                    }
                    case 'session.removed':
                        setSessions((list) => list.filter((s) => s.id !== msg.id));
                        break;
                    case 'machine.updated': {
                        const next = msg.machine;
                        setMachines((list) =>
                            list.some((m) => m.id === next.id)
                                ? list.map((m) => (m.id === next.id ? next : m))
                                : [...list, next],
                        );
                        break;
                    }
                    case 'output':
                        listeners.current.get(msg.id)?.forEach((l) =>
                            l({ type: 'output', data: msg.data, snapshot: msg.snapshot, cols: msg.cols, rows: msg.rows }),
                        );
                        break;
                    case 'watch':
                        listeners.current.get(msg.id)?.forEach((l) => l({ type: 'watch', live: msg.live }));
                        break;
                }
            };

            ws.onclose = () => {
                if (socket.current === ws) {
                    socket.current = null;
                }
                if (closed) {
                    return;
                }
                attempt += 1;
                setConnection('reconnecting');
                // A refused upgrade can mean the session ended: ask the server.
                void api
                    .get<{ signed_in: boolean }>('/api/state')
                    .then((s) => {
                        if (!s.signed_in) {
                            closed = true;
                            onSignedOut();
                        }
                    })
                    .catch(() => {});
                const delay = Math.min(30_000, 1000 * 2 ** Math.min(attempt - 1, 5));
                timer = window.setTimeout(connect, delay);
            };
        };

        connect();

        return () => {
            closed = true;
            window.clearTimeout(timer);
            socket.current?.close();
            socket.current = null;
        };
    }, [reload, onSignedOut]);

    const watch = useCallback(
        (id: string, listener: TerminalListener) => {
            let set = listeners.current.get(id);
            if (!set) {
                set = new Set();
                listeners.current.set(id, set);
                send({ type: 'watch', id, on: true });
            }
            set.add(listener);

            return () => {
                const current = listeners.current.get(id);
                if (!current) {
                    return;
                }
                current.delete(listener);
                if (current.size === 0) {
                    listeners.current.delete(id);
                    send({ type: 'watch', id, on: false });
                }
            };
        },
        [send],
    );

    const input = useCallback((id: string, data: string) => send({ type: 'input', id, data }), [send]);

    const act = useCallback(
        async (session: RemoteSession, action: SessionAction, body: Record<string, unknown> = {}, label = 'Sent') => {
            setPending((p) => ({ ...p, [session.id]: label }));
            try {
                await api.post(`/api/sessions/${encodeURIComponent(session.id)}/${action}`, body);

                return true;
            } catch (e) {
                setPending((p) => {
                    const rest = { ...p };
                    delete rest[session.id];

                    return rest;
                });
                toast.error(errorMessage(e, 'Could not reach the server.'));

                return false;
            }
        },
        [toast],
    );

    const answer = useCallback(
        (session: RemoteSession, choice: ApprovalChoice) =>
            act(session, 'answer', { choice }, choice === 'deny' ? 'Denying…' : 'Allowing…'),
        [act],
    );

    const value = useMemo<Live>(
        () => ({ connection, loaded, sessions, machines, pending, reload, act, answer, watch, input }),
        [connection, loaded, sessions, machines, pending, reload, act, answer, watch, input],
    );

    return <LiveContext.Provider value={value}>{children}</LiveContext.Provider>;
}
