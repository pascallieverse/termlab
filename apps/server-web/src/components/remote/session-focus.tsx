import { useState } from 'react';
import { ApprovalActions } from '@/components/el/approval-actions';
import { Button } from '@/components/el/button';
import { StatusPill } from '@/components/el/status';
import { agentName, compactNumber, timeAgo } from '@/lib/format';
import { cn } from '@/lib/utils';
import type { ApprovalChoice, RemoteSession } from '@/types';
import { LiveTerminal } from './live-terminal';

/** Focus view: the terminal, the approval bar, a message box and the inspector. */
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
    onAction: (
        action: 'message' | 'stop' | 'hold',
        body?: Record<string, unknown>,
        label?: string,
    ) => Promise<boolean>;
    onBack?: () => void;
}) {
    const [text, setText] = useState('');
    const [confirmStop, setConfirmStop] = useState(false);
    const needsYou = session.status === 'needs_you' && session.approval;
    /** Done and Error are final: there is no process left to stop. */
    const ended = session.status === 'done' || session.status === 'error';
    const ctx = session.context_pct ?? 0;

    async function send(e: React.FormEvent) {
        e.preventDefault();
        if (!text.trim()) {
            return;
        }
        if (await onAction('message', { text }, 'Message sent')) {
            setText('');
        }
    }

    return (
        <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
            <section
                aria-label="Terminal"
                className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 px-5 py-4 max-md:px-4"
            >
                <div className="flex flex-wrap items-center justify-between gap-2.5">
                    <div className="flex min-w-0 items-center gap-3">
                        {onBack && (
                            <button
                                type="button"
                                onClick={onBack}
                                aria-label="Back to sessions"
                                className="grid size-11 cursor-pointer place-items-center rounded-lg border border-line-strong text-ink hover:bg-raised"
                            >
                                <svg
                                    width="18"
                                    height="18"
                                    viewBox="0 0 24 24"
                                    fill="none"
                                    stroke="currentColor"
                                    strokeWidth="2"
                                    strokeLinecap="round"
                                    strokeLinejoin="round"
                                    aria-hidden="true"
                                >
                                    <path d="M15 18l-6-6 6-6" />
                                </svg>
                            </button>
                        )}
                        <div className="flex min-w-0 flex-col">
                            <h1 className="m-0 truncate font-display text-xl font-semibold">
                                {session.title}
                            </h1>
                            <span className="text-xs text-dim">
                                {session.project} · {agentName(session.agent)} ·{' '}
                                {session.machine}
                            </span>
                        </div>
                    </div>
                    <div className="flex flex-wrap items-center gap-1.5">
                        <StatusPill status={session.status} />
                        <Button
                            variant="outline"
                            className="min-h-10 rounded-[7px] px-3"
                            disabled={!online}
                            onClick={() =>
                                onAction('hold', {}, 'Putting on hold…')
                            }
                        >
                            Put on hold
                        </Button>
                        {confirmStop ? (
                            <>
                                <Button
                                    variant="danger"
                                    className="min-h-10 rounded-[7px] px-3"
                                    onClick={() => {
                                        setConfirmStop(false);
                                        void onAction('stop', {}, 'Stopping…');
                                    }}
                                >
                                    Stop the session
                                </Button>
                                <Button
                                    variant="ghost"
                                    className="min-h-10 rounded-[7px] px-3"
                                    onClick={() => setConfirmStop(false)}
                                >
                                    Keep running
                                </Button>
                            </>
                        ) : (
                            <Button
                                variant="danger"
                                className="min-h-10 rounded-[7px] px-3"
                                disabled={!online || ended}
                                onClick={() => setConfirmStop(true)}
                            >
                                Stop
                            </Button>
                        )}
                    </div>
                </div>

                {!online && (
                    <div
                        role="status"
                        className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-[10px] border border-line-strong bg-panel px-3.5 py-3 text-[13px] text-dim"
                    >
                        <span className="font-semibold text-ink">
                            {session.machine} is offline
                        </span>
                        <span>
                            · last seen {timeAgo(lastSeen)}. These are the last
                            lines it reported. Approvals and messages work again
                            once Termlab reconnects.
                        </span>
                    </div>
                )}

                <LiveTerminal
                    sessionId={session.id}
                    preview={session.preview}
                    interactive={online}
                />

                {needsYou && (
                    <div
                        role="alert"
                        className="flex flex-wrap items-center justify-between gap-3 rounded-[10px] border border-amber-line bg-amber-panel px-3.5 py-3"
                    >
                        <span className="flex min-w-0 flex-col gap-0.5">
                            <span className="font-semibold text-amber-soft">
                                {session.approval!.question}
                            </span>
                            {session.approval!.detail && (
                                <code className="font-mono text-[13px] break-all text-ink-soft">
                                    {session.approval!.detail}
                                </code>
                            )}
                        </span>
                        <ApprovalActions
                            onAnswer={onAnswer}
                            pending={
                                online
                                    ? pending
                                    : 'Answer when the machine is back online'
                            }
                        />
                    </div>
                )}

                <form onSubmit={send} className="flex gap-2">
                    <label htmlFor="msg" className="sr-only">
                        Message to the session
                    </label>
                    <input
                        id="msg"
                        type="text"
                        value={text}
                        onChange={(e) => setText(e.target.value)}
                        disabled={!online}
                        placeholder={
                            online
                                ? 'Type to the session… (sent as keystrokes)'
                                : `${session.machine} is offline`
                        }
                        className="min-h-11 min-w-0 flex-1 rounded-[9px] border border-line-strong bg-surface px-3 font-mono text-[13px] text-ink"
                    />
                    <Button
                        type="submit"
                        variant="light"
                        className="rounded-[9px]"
                        disabled={!online || !text.trim()}
                    >
                        Send
                    </Button>
                </form>
                {pending && !needsYou && (
                    <p role="status" className="m-0 text-[13px] text-dim">
                        {pending}
                    </p>
                )}
            </section>

            <aside
                aria-label="Inspector"
                className="flex flex-col gap-[18px] border-t border-line-soft bg-surface px-[18px] py-4 lg:w-[280px] lg:flex-none lg:overflow-y-auto lg:border-t-0 lg:border-l"
            >
                <h2 className="m-0 text-[13px] font-semibold tracking-[0.04em] text-dim">
                    INSPECTOR
                </h2>
                <dl className="m-0 grid grid-cols-2 gap-x-2.5 gap-y-3 text-[13px]">
                    <div>
                        <dt className="text-dim">Model</dt>
                        <dd className="m-0">{session.model ?? '—'}</dd>
                    </div>
                    <div>
                        <dt className="text-dim">Branch</dt>
                        <dd className="m-0 truncate font-mono text-xs">
                            {session.branch ?? '—'}
                        </dd>
                    </div>
                    <div>
                        <dt className="text-dim">Agent</dt>
                        <dd className="m-0">{agentName(session.agent)}</dd>
                    </div>
                    <div>
                        <dt className="text-dim">Tokens</dt>
                        <dd className="m-0">{compactNumber(session.tokens)}</dd>
                    </div>
                </dl>
                {session.context_pct !== null && (
                    <div className="flex flex-col gap-1.5">
                        <span className="flex justify-between text-[13px]">
                            <span className="text-dim">Context window</span>
                            <span>{ctx}%</span>
                        </span>
                        <div
                            role="meter"
                            aria-label="Context window used"
                            aria-valuenow={ctx}
                            aria-valuemin={0}
                            aria-valuemax={100}
                            className="h-1.5 overflow-hidden rounded-full bg-line-soft"
                        >
                            <div
                                className={cn(
                                    'h-full',
                                    ctx > 80 ? 'bg-bad-dot' : 'bg-ink',
                                )}
                                style={{ width: `${ctx}%` }}
                            />
                        </div>
                    </div>
                )}
                <div className="flex flex-col gap-2">
                    <span className="text-[13px] text-dim">Changed files</span>
                    {session.changed_files.length === 0 && (
                        <span className="text-[13px] text-faint">None yet</span>
                    )}
                    {session.changed_files.map((file) => (
                        <span
                            key={file.path}
                            className="flex justify-between gap-2 font-mono text-xs"
                        >
                            <span className="truncate">{file.path}</span>
                            <span className="flex-none">
                                <span className="text-ok">
                                    +{file.added ?? 0}
                                </span>{' '}
                                <span className="text-bad">
                                    −{file.removed ?? 0}
                                </span>
                            </span>
                        </span>
                    ))}
                </div>
                <p className="mt-auto mb-0 text-xs text-dim">
                    Remote view of the session running on {session.machine}.
                </p>
            </aside>
        </div>
    );
}
