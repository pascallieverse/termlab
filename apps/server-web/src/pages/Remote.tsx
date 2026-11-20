import { Link, useSearchParams } from 'react-router-dom';
import { useMemo, useState } from 'react';
import { PushToggle } from '@/components/push-toggle';
import { ApprovalActions } from '@/components/el/approval-actions';
import { Segmented } from '@/components/el/segmented';
import { StatusDot, StatusPill } from '@/components/el/status';
import { TermlabIcon } from '@/components/el/logo';
import { SessionFocus } from '@/components/remote/session-focus';
import { useNarrow } from '@/components/remote/use-narrow';
import { BottomNav, NAV } from '@/layouts/nav';
import { useLive } from '@/live/LiveProvider';
import { agentName, agentShort, timeAgo } from '@/lib/format';
import { cn } from '@/lib/utils';
import type { Machine, RemoteSession } from '@/types';

type Filter = 'all' | 'attention' | 'running' | 'done';
type View = 'grid' | 'focus';

const FILTERS: Record<Filter, (s: RemoteSession) => boolean> = {
    all: () => true,
    attention: (s) => s.status === 'needs_you' || s.status === 'error',
    running: (s) => s.status === 'running',
    done: (s) => s.status === 'done',
};

export default function Remote() {
    const [params] = useSearchParams();
    const live = useLive();
    const narrow = useNarrow();

    const [machineId, setMachineId] = useState<number | 'all'>('all');
    const [filter, setFilter] = useState<Filter>('all');
    const [selected, setSelected] = useState<string | null>(
        params.get('session'),
    );
    const [view, setView] = useState<View>(
        params.get('session') ? 'focus' : 'grid',
    );

    const sessions = useMemo(
        () =>
            live.sessions.filter(
                (s) => machineId === 'all' || s.machine_id === machineId,
            ),
        [live.sessions, machineId],
    );
    const current =
        live.sessions.find((s) => s.id === selected) ?? sessions[0] ?? null;
    const online = (s: RemoteSession) =>
        live.machines.find((m) => m.id === s.machine_id)?.online ?? false;

    const open = (s: RemoteSession) => {
        setSelected(s.id);
        setView('focus');
    };

    const shared = {
        live,
        sessions,
        machines: live.machines,
        machineId,
        setMachineId,
        open,
        online,
    };

    const focus = current && (
        <SessionFocus
            key={current.id}
            session={current}
            online={online(current)}
            lastSeen={
                live.machines.find((m) => m.id === current.machine_id)
                    ?.last_seen_at
            }
            pending={live.pending[current.id]}
            onAnswer={(choice) => live.answer(current, choice)}
            onAction={(action, body, label) =>
                live.act(current, action, body, label)
            }
            onBack={narrow ? () => setView('grid') : undefined}
        />
    );

    return (
        <>
            {narrow ? (
                view === 'focus' && focus ? (
                    <div className="flex min-h-screen flex-col bg-bg text-sm text-ink">
                        {focus}
                    </div>
                ) : (
                    <PhoneLayout {...shared} />
                )
            ) : (
                <DesktopLayout
                    {...shared}
                    filter={filter}
                    setFilter={setFilter}
                    view={view}
                    setView={setView}
                    current={current}
                    focus={focus}
                />
            )}
        </>
    );
}

type Shared = {
    live: ReturnType<typeof useLive>;
    sessions: RemoteSession[];
    machines: Machine[];
    machineId: number | 'all';
    setMachineId: (id: number | 'all') => void;
    open: (s: RemoteSession) => void;
    online: (s: RemoteSession) => boolean;
};

function MachineSelect({
    machines,
    machineId,
    setMachineId,
    compact = false,
}: Pick<Shared, 'machines' | 'machineId' | 'setMachineId'> & {
    compact?: boolean;
}) {
    const selectedMachine = machines.find((m) => m.id === machineId);
    const dotOn = selectedMachine
        ? selectedMachine.online
        : machines.some((m) => m.online);

    return (
        <label
            className={cn(
                'flex items-center gap-2 text-dim',
                compact &&
                    'rounded-full border border-line-strong bg-panel pl-3 text-[13px]',
            )}
        >
            <span className="sr-only">Machine</span>
            <span
                aria-hidden="true"
                className={cn(
                    'size-2 rounded-full',
                    dotOn ? 'bg-ok' : 'bg-faint',
                )}
            />
            <select
                value={machineId}
                onChange={(e) =>
                    setMachineId(
                        e.target.value === 'all'
                            ? 'all'
                            : Number(e.target.value),
                    )
                }
                className={cn(
                    'min-h-9 cursor-pointer text-ink',
                    compact
                        ? 'rounded-full bg-transparent pr-3'
                        : 'rounded-lg border border-line-strong bg-panel px-2.5',
                )}
            >
                <option value="all">All machines</option>
                {machines.map((m) => (
                    <option key={m.id} value={m.id}>
                        {m.name}
                        {m.online ? '' : ' (offline)'}
                    </option>
                ))}
            </select>
        </label>
    );
}

function ConnectionBadge() {
    const status = useLive().connection;
    const ok = status === 'connected';

    return (
        <span className="flex items-center gap-1.5 font-mono text-xs text-dim">
            <span
                aria-hidden="true"
                className={cn(
                    'size-1.5 rounded-full',
                    ok ? 'bg-ok' : 'bg-amber',
                )}
            />
            {ok ? 'connected' : status}
        </span>
    );
}

/** The other sections, next to the connection badge on wide screens. */
function SectionLinks() {
    return (
        <nav aria-label="Sections" className="flex items-center gap-1">
            {NAV.filter((n) => n.to !== '/remote').map((n) => (
                <Link
                    key={n.to}
                    to={n.to}
                    className="rounded-lg px-2.5 py-1.5 text-[13px] text-dim no-underline hover:bg-raised hover:text-ink"
                >
                    {n.label}
                </Link>
            ))}
        </nav>
    );
}

function EmptyState({ machines }: { machines: Machine[] }) {
    return (
        <div className="m-5 flex flex-col items-center gap-2 rounded-2xl border border-dashed border-line-strong px-5 py-10 text-center">
            <span className="font-semibold">
                {machines.length === 0
                    ? 'No machines paired yet'
                    : 'No sessions here'}
            </span>
            <span className="max-w-[44ch] text-sm text-dim">
                {machines.length === 0 ? (
                    <>
                        Pair Termlab on the{' '}
                        <Link to="/machines" className="text-amber">
                            Machines
                        </Link>{' '}
                        page, then its sessions show up here.
                    </>
                ) : (
                    'Sessions that Termlab runs on your machines appear here as soon as they start.'
                )}
            </span>
        </div>
    );
}

function DesktopLayout({
    live,
    sessions,
    machines,
    machineId,
    setMachineId,
    open,
    online,
    filter,
    setFilter,
    view,
    setView,
    current,
    focus,
}: Shared & {
    filter: Filter;
    setFilter: (f: Filter) => void;
    view: View;
    setView: (v: View) => void;
    current: RemoteSession | null;
    focus: React.ReactNode;
}) {
    const attention = sessions.filter((s) => s.status === 'needs_you').length;
    const visible = sessions.filter(FILTERS[filter]);
    const projects = [...new Set(sessions.map((s) => s.project))];

    return (
        <div className="flex h-screen flex-col overflow-hidden bg-bg text-sm leading-normal text-ink">
            <header className="flex flex-wrap items-center gap-x-5 gap-y-2.5 border-b border-line-soft bg-surface px-4 py-2.5">
                <Link
                    to="/remote"
                    className="flex items-center gap-2.5 text-ink no-underline hover:text-ink"
                >
                    <TermlabIcon className="size-[26px] rounded-md" />
                    <span className="font-display text-base font-semibold">
                        Termlab <span className="text-amber">Server</span>
                    </span>
                </Link>
                <MachineSelect
                    machines={machines}
                    machineId={machineId}
                    setMachineId={setMachineId}
                />
                <Segmented
                    label="Filter sessions"
                    size="sm"
                    value={filter}
                    onChange={setFilter}
                    className="rounded-[10px] p-[3px]"
                    options={[
                        { value: 'all', label: 'All' },
                        {
                            value: 'attention',
                            label: `Attention · ${attention}`,
                        },
                        { value: 'running', label: 'Running' },
                        { value: 'done', label: 'Done' },
                    ]}
                />
                <div className="ml-auto flex items-center gap-3.5">
                    <ConnectionBadge />
                    <SectionLinks />
                </div>
            </header>

            <div className="flex min-h-0 flex-1">
                <nav
                    aria-label="Projects"
                    className="flex w-[250px] flex-none flex-col gap-3.5 overflow-y-auto border-r border-line-soft bg-surface px-2.5 py-3.5"
                >
                    {projects.length === 0 && (
                        <span className="px-2 text-[13px] text-faint">
                            No sessions
                        </span>
                    )}
                    {projects.map((project) => {
                        const list = sessions.filter(
                            (s) => s.project === project,
                        );
                        const n = list.filter(
                            (s) => s.status === 'needs_you',
                        ).length;

                        return (
                            <div
                                key={project}
                                className="flex flex-col gap-0.5"
                            >
                                <div className="flex items-center justify-between px-2 py-1">
                                    <span className="text-xs font-semibold tracking-[0.04em] text-dim uppercase">
                                        {project}
                                    </span>
                                    {n > 0 && (
                                        <span className="min-w-[18px] rounded-full bg-amber px-1.5 py-px text-center text-[11px] font-bold text-amber-ink">
                                            {n}
                                        </span>
                                    )}
                                </div>
                                {list.map((s) => {
                                    const isCur =
                                        view === 'focus' &&
                                        current?.id === s.id;

                                    return (
                                        <button
                                            key={s.id}
                                            type="button"
                                            onClick={() => open(s)}
                                            aria-current={
                                                isCur ? 'true' : undefined
                                            }
                                            className={cn(
                                                'flex min-h-9 w-full cursor-pointer items-center gap-2.5 rounded-[7px] px-2 text-left text-[13px] text-ink',
                                                isCur
                                                    ? 'bg-raised'
                                                    : 'hover:bg-raised/60',
                                            )}
                                        >
                                            <StatusDot status={s.status} />
                                            <span className="min-w-0 flex-1 truncate">
                                                {s.title}
                                            </span>
                                            <span className="text-[11px] text-dim">
                                                {agentShort(s.agent)}
                                            </span>
                                        </button>
                                    );
                                })}
                            </div>
                        );
                    })}
                </nav>

                <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">
                    <div className="flex flex-wrap items-center justify-between gap-3 border-b border-line-soft px-5 py-3.5">
                        <Segmented
                            label="View"
                            size="sm"
                            value={view}
                            onChange={setView}
                            className="rounded-[10px] p-[3px]"
                            options={[
                                { value: 'grid', label: 'Mission control' },
                                { value: 'focus', label: 'Focus' },
                            ]}
                        />
                        <span className="text-dim">
                            {sessions.length} sessions · {attention} waiting on
                            you
                        </span>
                    </div>

                    {view === 'grid' || !focus ? (
                        visible.length === 0 ? (
                            <EmptyState machines={machines} />
                        ) : (
                            <div className="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] content-start gap-3.5 p-5">
                                {visible.map((s) => (
                                    <article
                                        key={s.id}
                                        className={cn(
                                            'flex flex-col gap-2.5 rounded-xl border bg-panel p-3.5',
                                            s.status === 'needs_you'
                                                ? 'border-amber-line'
                                                : 'border-line',
                                        )}
                                    >
                                        <button
                                            type="button"
                                            onClick={() => open(s)}
                                            className="flex min-h-11 cursor-pointer flex-col gap-1 text-left text-ink"
                                        >
                                            <span className="flex w-full items-center justify-between gap-2">
                                                <span className="font-semibold">
                                                    {s.title}
                                                </span>
                                                <StatusPill status={s.status} />
                                            </span>
                                            <span className="text-xs text-dim">
                                                {s.project} ·{' '}
                                                {agentName(s.agent)}
                                                {machineId === 'all' &&
                                                    ` · ${s.machine}`}
                                            </span>
                                        </button>
                                        <pre className="m-0 h-[78px] overflow-hidden rounded-lg border border-line-soft bg-bg p-2.5 font-mono text-xs leading-[1.6] whitespace-pre-wrap text-dim">
                                            {s.preview
                                                .filter((line) => line?.trim())
                                                .slice(-3)
                                                .join('\n') || ' '}
                                        </pre>
                                        {s.status === 'needs_you' &&
                                            s.approval && (
                                                <div className="flex flex-col gap-2">
                                                    <span className="text-[13px] text-amber-soft">
                                                        {s.approval.question}
                                                    </span>
                                                    <ApprovalActions
                                                        size="sm"
                                                        pending={
                                                            online(s)
                                                                ? live.pending[
                                                                      s.id
                                                                  ]
                                                                : 'Machine offline'
                                                        }
                                                        onAnswer={(c) =>
                                                            live.answer(s, c)
                                                        }
                                                    />
                                                </div>
                                            )}
                                        {!online(s) && (
                                            <span className="text-xs text-faint">
                                                {s.machine} is offline
                                            </span>
                                        )}
                                    </article>
                                ))}
                            </div>
                        )
                    ) : (
                        focus
                    )}
                </main>
            </div>
        </div>
    );
}

function PhoneLayout({
    live,
    sessions,
    machines,
    machineId,
    setMachineId,
    open,
    online,
}: Shared) {
    const [tab, setTab] = useState<'queue' | 'all'>('queue');
    const queue = sessions.filter((s) => s.status === 'needs_you');

    return (
        <div className="flex h-dvh flex-col overflow-hidden bg-bg text-[15px] leading-[1.45] text-ink">
            <header className="flex flex-col gap-3.5 px-4 pt-5 pb-3">
                <div className="flex items-center justify-between gap-3">
                    <Link
                        to="/remote"
                        className="flex items-center gap-2.5 text-ink no-underline hover:text-ink"
                    >
                        <TermlabIcon className="size-7 rounded-[7px]" />
                        <span className="font-display text-lg font-semibold">
                            Termlab <span className="text-amber">Server</span>
                        </span>
                    </Link>
                    <MachineSelect
                        compact
                        machines={machines}
                        machineId={machineId}
                        setMachineId={setMachineId}
                    />
                </div>
                <Segmented
                    label="Show"
                    size="sm"
                    stretch
                    value={tab}
                    onChange={setTab}
                    className="rounded-[11px] p-[3px] [&_button]:min-h-10 [&_button]:text-sm"
                    options={[
                        {
                            value: 'queue',
                            label: `Needs you · ${queue.length}`,
                        },
                        { value: 'all', label: 'All sessions' },
                    ]}
                />
            </header>

            <main className="flex flex-1 flex-col gap-3 overflow-y-auto px-4 pt-1 pb-4">
                {tab === 'queue' &&
                    queue.map((s) => (
                        <article
                            key={s.id}
                            className="flex flex-col gap-3 rounded-2xl border border-amber-line bg-amber-panel p-4"
                        >
                            <div className="flex items-center justify-between gap-2">
                                <span className="text-xs text-dim">
                                    {s.project} · {agentName(s.agent)}
                                </span>
                                <span className="text-[11px] text-dim">
                                    {timeAgo(s.status_changed_at)}
                                </span>
                            </div>
                            <h2 className="m-0 text-[17px] font-semibold">
                                {s.approval?.question ??
                                    'Waiting for your input'}
                            </h2>
                            {s.approval?.detail && (
                                <pre className="m-0 rounded-[10px] border border-line bg-bg p-3 font-mono text-[13px] whitespace-pre-wrap text-ink-soft">
                                    {s.approval.detail}
                                </pre>
                            )}
                            {s.approval && (
                                <ApprovalActions
                                    layout="stack"
                                    pending={
                                        online(s)
                                            ? live.pending[s.id]
                                            : `${s.machine} is offline`
                                    }
                                    onAnswer={(c) => live.answer(s, c)}
                                />
                            )}
                            <button
                                type="button"
                                onClick={() => open(s)}
                                className="cursor-pointer py-1.5 text-center text-sm text-amber hover:text-amber-soft"
                            >
                                Open terminal
                            </button>
                        </article>
                    ))}

                {tab === 'queue' && queue.length === 0 && (
                    <div className="flex flex-col items-center gap-1.5 rounded-2xl border border-dashed border-line-strong px-5 py-8 text-center">
                        <svg
                            width="28"
                            height="28"
                            viewBox="0 0 24 24"
                            aria-hidden="true"
                            fill="none"
                            stroke="#8EE6B4"
                            strokeWidth="2"
                            strokeLinecap="round"
                            strokeLinejoin="round"
                        >
                            <path d="M5 12.5l4.5 4.5L19 7.5" />
                        </svg>
                        <span className="font-semibold">
                            Nothing waiting on you
                        </span>
                        <span className="text-sm text-dim">
                            We will notify you when a session needs you.
                        </span>
                        <PushToggle compact className="mt-2" />
                    </div>
                )}

                {tab === 'all' &&
                    (sessions.length === 0 ? (
                        <EmptyState machines={machines} />
                    ) : (
                        <ul className="m-0 flex list-none flex-col overflow-hidden rounded-[14px] border border-line bg-panel p-0">
                            {sessions.map((s, i) => (
                                <li
                                    key={s.id}
                                    className={cn(
                                        i > 0 && 'border-t border-line-soft',
                                    )}
                                >
                                    <button
                                        type="button"
                                        onClick={() => open(s)}
                                        className="flex min-h-11 w-full cursor-pointer items-center gap-3 px-3.5 py-3 text-left text-ink"
                                    >
                                        <StatusDot
                                            status={s.status}
                                            className="size-[9px]"
                                        />
                                        <span className="flex min-w-0 flex-1 flex-col">
                                            <span className="truncate font-medium">
                                                {s.title}
                                            </span>
                                            <span className="text-xs text-dim">
                                                {s.project} ·{' '}
                                                {agentName(s.agent)}
                                            </span>
                                        </span>
                                        <StatusPill status={s.status} />
                                    </button>
                                </li>
                            ))}
                        </ul>
                    ))}
            </main>

            <BottomNav />
        </div>
    );
}
