import { useEffect, useState, useSyncExternalStore } from 'react';
import { Monogram } from '@/components/el/agent';
import { ApprovalActions } from '@/components/el/approval-actions';
import { Button } from '@/components/el/button';
import { StatusPill } from '@/components/el/status';
import { agentName, atLeast, compactNumber, timeAgo } from '@/lib/format';
import { useLive } from '@/live/LiveProvider';
import type { ApprovalChoice, RemoteSession } from '@/types';
import { LiveTerminal } from './live-terminal';
import { ProjectNotes } from './project-notes';

// ---- the right column: open or closed, on the session's details or its project's notes ----
// As in the desktop app; remembered per browser (a convenience, so storage may fail).

type InspectorTab = 'session' | 'notes';
type InspectorState = { open: boolean; tab: InspectorTab };

const KEY = 'termlab.inspector';
const WIDE = '(min-width: 1024px)';
const listeners = new Set<() => void>();
let state: InspectorState | null = null;

function readState(): InspectorState {
    if (state) return state;
    let saved: Partial<InspectorState> = {};
    try {
        saved = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<InspectorState>;
    } catch {
        // Storage unavailable: defaults.
    }
    // Closed until you open it, as in the desktop app; then remembered. On narrow screens it
    // covers the terminal, so it starts closed there.
    state = {
        open: saved.open === true && window.matchMedia(WIDE).matches,
        tab: saved.tab === 'notes' ? 'notes' : 'session',
    };
    return state;
}

function setState(next: InspectorState) {
    state = next;
    try {
        localStorage.setItem(KEY, JSON.stringify(next));
    } catch {
        // Still works for this page.
    }
    listeners.forEach((l) => l());
}

function useInspector(): [InspectorState, (tab: InspectorTab) => void, () => void] {
    const s = useSyncExternalStore(
        (cb) => {
            listeners.add(cb);
            return () => listeners.delete(cb);
        },
        readState,
        readState,
    );
    /** Opens on a tab, or closes when that tab is already showing. */
    const show = (tab: InspectorTab) => setState({ open: !(s.open && s.tab === tab), tab });
    const close = () => setState({ ...s, open: false });
    return [s, show, close];
}

function useWide(): boolean {
    return useSyncExternalStore(
        (cb) => {
            const mql = window.matchMedia(WIDE);
            mql.addEventListener('change', cb);
            return () => mql.removeEventListener('change', cb);
        },
        () => window.matchMedia(WIDE).matches,
        () => true,
    );
}

function Icon({ name }: { name: 'note' | 'panel' | 'back' | 'close' }) {
    const paths = {
        note: (
            <>
                <path d="M5 4h14v11l-5 5H5z" />
                <path d="M14 20v-5h5M8.5 9h7M8.5 12.5h4" />
            </>
        ),
        panel: (
            <>
                <rect x="3" y="4" width="18" height="16" rx="2" />
                <path d="M15 4v16" />
            </>
        ),
        back: <path d="M15 18l-6-6 6-6" />,
        close: <path d="M18 6 6 18M6 6l12 12" />,
    };
    return (
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            {paths[name]}
        </svg>
    );
}

/** "/Users/pascal/Code/api" → "~/Code/api". */
function tildify(path: string): string {
    return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, '~').replace(/^[A-Z]:\\Users\\[^\\]+/i, '~');
}

/** Focus view, laid out like the desktop app's session pane: header, terminal, approval, inspector. */
export function SessionFocus({
    session,
    online,
    lastSeen,
    pending,
    onAnswer,
    onAction,
    onBack,
}: {
    session: RemoteSession;
    online: boolean;
    lastSeen?: string | null;
    pending?: string;
    onAnswer: (choice: ApprovalChoice) => void;
    onAction: (action: 'message' | 'stop' | 'hold' | 'unhold', body?: Record<string, unknown>, label?: string) => Promise<boolean>;
    onBack?: () => void;
}) {
    const [inspector, show, close] = useInspector();
    const live = useLive();
    const canResize = atLeast(live.machines.find((m) => m.id === session.machine_id)?.version, '0.2.9');
    const wide = useWide();
    const approval = session.status === 'needs_you' ? session.approval : null;
    const agent = agentName(session.agent);

    // On narrow screens the panel covers the terminal: Escape closes it.
    useEffect(() => {
        if (wide || !inspector.open) return;
        const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close();
        window.addEventListener('keydown', onKey);
        return () => window.removeEventListener('keydown', onKey);
    }, [wide, inspector.open, close]);

    const toggle = (tab: InspectorTab, label: string) => {
        const on = inspector.open && inspector.tab === tab;
        return (
            <button
                type="button"
                className={`icon-btn focus-toggle${on ? ' on' : ''}`}
                aria-label={label}
                aria-pressed={on}
                title={`${on ? 'Hide' : 'Show'} ${label.toLowerCase()}`}
                onClick={() => show(tab)}
            >
                <Icon name={tab === 'notes' ? 'note' : 'panel'} />
            </button>
        );
    };

    return (
        <div className="relative flex min-h-0 flex-1">
            <section aria-label={session.title} className="flex min-h-0 min-w-0 flex-1 flex-col">
                <div className="session-head">
                    {onBack && (
                        <button type="button" onClick={onBack} aria-label="Back to sessions" className="icon-btn focus-toggle -ml-2">
                            <Icon name="back" />
                        </button>
                    )}
                    <span className="max-md:hidden">
                        <Monogram agent={session.agent} size={38} />
                    </span>
                    <span className="md:hidden">
                        <Monogram agent={session.agent} size={30} />
                    </span>
                    <div className="session-title">
                        <h1 title={session.title}>{session.title}</h1>
                        <div className="session-meta">
                            <span className="max-md:hidden">{agent}</span>
                            <span aria-hidden="true" className="max-md:hidden">
                                ·
                            </span>
                            <span className="truncate font-mono">{session.model ?? 'default model'}</span>
                            {session.branch && (
                                <>
                                    <span aria-hidden="true">·</span>
                                    <span className="truncate font-mono">⎇ {session.branch}</span>
                                </>
                            )}
                        </div>
                    </div>
                    <span className="max-md:hidden">
                        <StatusPill status={session.status} />
                    </span>
                    <div className="flex-1" />
                    {toggle('notes', 'Project notes')}
                    {toggle('session', 'Session details')}
                </div>

                {!online && (
                    <div role="status" className="banner muted">
                        <span>
                            <strong className="font-semibold text-ink">{session.machine} is offline</strong> · last seen {timeAgo(lastSeen)}. These
                            are the last lines it reported. Approvals work again once Termlab reconnects.
                        </span>
                    </div>
                )}

                <div className="flex min-h-0 flex-1 flex-col bg-term">
                    <LiveTerminal sessionId={session.id} preview={session.preview} interactive={online} canResize={canResize} />
                </div>

                {approval && (
                    <div role="alert" aria-label={approval.kind === 'question' ? 'Question' : 'Permission request'} className="approval">
                        <div className="approval-text">
                            <span className="approval-title">
                                {agent} {approval.kind === 'question' ? 'has a question for you' : 'is asking for permission'}
                            </span>
                            {approval.detail && <code>{approval.detail}</code>}
                            {approval.question && <span className="approval-question">{approval.question}</span>}
                        </div>
                        <ApprovalActions onAnswer={onAnswer} pending={online ? pending : 'Answer when the machine is back online'} />
                    </div>
                )}
                {pending && !approval && (
                    <p role="status" className="m-0 px-5 py-2 text-xs text-dim">
                        {pending}
                    </p>
                )}
            </section>

            {inspector.open && (
                <>
                    {!wide && <button type="button" aria-label="Close the panel" className="inspector-scrim" onClick={close} />}
                    <aside
                        aria-label={inspector.tab === 'notes' ? 'Project notes' : 'Session details'}
                        className={`inspector${wide ? '' : ' inspector-sheet'}`}
                    >
                        {!wide && (
                            <div className="inspector-sheet-head">
                                <div role="tablist" aria-label="Panel" className="segmented">
                                    {(['session', 'notes'] as const).map((t) => (
                                        <button key={t} type="button" role="tab" aria-selected={inspector.tab === t} className={inspector.tab === t ? 'on' : ''} onClick={() => setState({ open: true, tab: t })}>
                                            {t === 'session' ? 'Session' : 'Notes'}
                                        </button>
                                    ))}
                                </div>
                                <button type="button" className="icon-btn focus-toggle" aria-label="Close" onClick={close}>
                                    <Icon name="close" />
                                </button>
                            </div>
                        )}
                        {inspector.tab === 'notes' ? (
                            <ProjectNotes key={`${session.machine_id}:${session.directory ?? session.project}`} session={session} online={online} />
                        ) : (
                            <SessionDetails session={session} online={online} onAction={onAction} />
                        )}
                    </aside>
                </>
            )}
        </div>
    );
}

function SessionDetails({
    session,
    online,
    onAction,
}: {
    session: RemoteSession;
    online: boolean;
    onAction: (action: 'message' | 'stop' | 'hold' | 'unhold', body?: Record<string, unknown>, label?: string) => Promise<boolean>;
}) {
    const [confirmStop, setConfirmStop] = useState(false);
    /** Done and Error are final: there is no process left to stop. */
    const ended = session.status === 'done' || session.status === 'error';
    const ctx = session.context_pct;
    const ctxColor = ctx === null ? 'var(--color-info-dot)' : ctx > 80 ? 'var(--color-bad-dot)' : ctx > 60 ? 'var(--color-amber)' : 'var(--color-info-dot)';
    const files = session.changed_files;

    return (
        <>
            <section>
                <h2 className="section-heading">Session</h2>
                <dl className="facts">
                    <dt>Agent</dt>
                    <dd>{agentName(session.agent)}</dd>
                    <dt>Status</dt>
                    <dd className="flex flex-wrap items-center gap-2">
                        <StatusPill status={session.status} />
                        {session.on_hold && <span className="text-xs text-dim">On hold</span>}
                    </dd>
                    <dt>Model</dt>
                    <dd className="mono">{session.model ?? 'default'}</dd>
                    <dt>Directory</dt>
                    <dd className="mono">{session.directory ? tildify(session.directory) : session.project}</dd>
                    <dt>Branch</dt>
                    <dd className="mono">{session.branch ?? '—'}</dd>
                    <dt>Machine</dt>
                    <dd>
                        <span className="inline-flex items-center gap-1.5">
                            <span aria-hidden="true" className="dot" style={{ background: online ? '#3FB950' : '#8B929C' }} />
                            {session.machine}
                            <span className="sr-only">{online ? '(online)' : '(offline)'}</span>
                        </span>
                    </dd>
                </dl>
            </section>

            <section>
                <h2 className="section-heading">Context &amp; spend</h2>
                <div className="row-between">
                    <span className="text-dim">Context window</span>
                    <span>{ctx === null ? '—' : `${ctx}%`}</span>
                </div>
                <div className="meter" role="img" aria-label={ctx === null ? 'Context window use unknown' : `Context window ${ctx} percent used`}>
                    <div style={{ width: `${ctx ?? 0}%`, background: ctxColor }} />
                </div>
                <div className="stat-grid">
                    <div className="stat">
                        <div className="stat-label">Tokens</div>
                        <div className="stat-value">{compactNumber(session.tokens)}</div>
                    </div>
                    <div className="stat">
                        <div className="stat-label">Changed files</div>
                        <div className="stat-value">{files.length}</div>
                    </div>
                </div>
            </section>

            <section>
                <h2 className="section-heading">Changed files</h2>
                {files.length === 0 && <p className="m-0 text-xs text-dim">No changes yet.</p>}
                {files.slice(0, 30).map((file) => (
                    <div key={file.path} className="file-row">
                        <span className="file-path" title={file.path}>
                            {file.path}
                        </span>
                        <span className="text-ok">+{file.added ?? 0}</span>
                        <span className="text-[#ff7b72]">−{file.removed ?? 0}</span>
                    </div>
                ))}
                {files.length > 30 && <p className="m-0 text-xs text-faint">{files.length - 30} more…</p>}
            </section>

            <section>
                <h2 className="section-heading">Actions</h2>
                {session.on_hold ? (
                    <Button variant="default" disabled={!online} onClick={() => onAction('unhold', {}, 'Taking off hold…')}>
                        Take off hold
                    </Button>
                ) : (
                    <Button variant="default" disabled={!online || ended} onClick={() => onAction('hold', {}, 'Putting on hold…')}>
                        Put on hold
                    </Button>
                )}
                {confirmStop ? (
                    <div className="grid grid-cols-2 gap-2">
                        <Button
                            variant="danger"
                            onClick={() => {
                                setConfirmStop(false);
                                void onAction('stop', {}, 'Stopping…');
                            }}
                        >
                            Stop it
                        </Button>
                        <Button variant="ghost" onClick={() => setConfirmStop(false)}>
                            Keep running
                        </Button>
                    </div>
                ) : (
                    <Button variant="danger" disabled={!online || ended} onClick={() => setConfirmStop(true)}>
                        Stop session
                    </Button>
                )}
                {!online && <p className="m-0 text-xs text-faint">{session.machine} is offline.</p>}
            </section>
        </>
    );
}
