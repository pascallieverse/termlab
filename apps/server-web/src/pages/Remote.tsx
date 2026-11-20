import { Link, useSearchParams } from 'react-router-dom';
import { useMemo, useState, type ReactNode } from 'react';
import { PushToggle } from '@/components/push-toggle';
import { Monogram } from '@/components/el/agent';
import { ApprovalActions } from '@/components/el/approval-actions';
import { Button } from '@/components/el/button';
import { Segmented } from '@/components/el/segmented';
import { StatusDot, StatusPill, StatusText } from '@/components/el/status';
import { NewSessionDialog } from '@/components/remote/new-session-dialog';
import { SessionFocus } from '@/components/remote/session-focus';
import { UsageChip } from '@/components/remote/usage-chip';
import { useNarrow } from '@/components/remote/use-narrow';
import { BottomNav, TopBar } from '@/layouts/nav';
import { useLive } from '@/live/LiveProvider';
import { STATUS, agentName, relativeTime, timeAgo } from '@/lib/format';
import { cn } from '@/lib/utils';
import type { Machine, MachineProjects, RemoteSession, SessionStatus } from '@/types';

type Filter = 'all' | 'attention' | 'running' | 'on_hold';
type View = 'grid' | 'focus';

// As in the desktop app (sessionFilters.ts): held sessions are only under On hold.
const FILTERS: Record<Filter, { label: string; test: (s: RemoteSession) => boolean }> = {
    all: { label: 'All', test: (s) => !s.on_hold },
    attention: { label: 'Attention', test: (s) => needsAttention(s) },
    running: { label: 'Running', test: (s) => !s.on_hold && s.status === 'running' },
    on_hold: { label: 'On hold', test: (s) => !!s.on_hold },
};

function needsAttention(s: RemoteSession): boolean {
    return !s.on_hold && (s.status === 'needs_you' || s.status === 'error');
}

/** A project in the sidebar: one of a machine's folders (in Termlab's order), with its sessions. */
type Group = {
    key: string;
    name: string;
    machineId: number;
    /** The folder's id; null for sessions from a Termlab that doesn't send its projects. */
    folderId: number | null;
    list: RemoteSession[];
};

/**
 * The sidebar's projects: every project folder of the shown machine(s), in Termlab's order,
 * also the ones without sessions, as in the app. Sessions newest first.
 */
function projectGroups(visible: RemoteSession[], machines: Machine[], projects: Record<number, MachineProjects>, machineId: number | 'all', keepEmpty: boolean): Group[] {
    const out: Group[] = [];
    const placed = new Set<string>();
    const newest = (l: RemoteSession[]) => [...l].sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0));
    for (const m of machines) {
        if (machineId !== 'all' && m.id !== machineId) continue;
        const mine = visible.filter((s) => s.machine_id === m.id);
        for (const p of projects[m.id]?.projects ?? []) {
            const list = mine.filter((s) => (s.folder_id != null ? s.folder_id === p.id : s.project === p.name));
            list.forEach((s) => placed.add(s.id));
            if (list.length > 0 || keepEmpty) {
                out.push({ key: `${m.id}:${p.id}`, name: p.name, machineId: m.id, folderId: p.id, list: newest(list) });
            }
        }
        // From a Termlab that doesn't send its projects (before 0.2.9).
        for (const [name, list] of byProject(mine.filter((s) => !placed.has(s.id)))) {
            out.push({ key: `${m.id}:name:${name}`, name, machineId: m.id, folderId: null, list });
        }
    }

    return out;
}

/** When the session last changed, for "2m" in the list. */
function lastActivity(s: RemoteSession): string | null {
    return s.status_changed_at ?? s.updated_at;
}

export default function Remote() {
    const [params] = useSearchParams();
    const live = useLive();
    const narrow = useNarrow();

    const [machineId, setMachineId] = useState<number | 'all'>('all');
    const [filter, setFilter] = useState<Filter>('all');
    const [query, setQuery] = useState('');
    const [selected, setSelected] = useState<string | null>(params.get('session'));
    // Focus first, as in Termlab. On a phone the session list comes first; a session opens from there.
    const [view, setView] = useState<View>(params.get('session') || !narrow ? 'focus' : 'grid');

    const onMachine = useMemo(() => live.sessions.filter((s) => machineId === 'all' || s.machine_id === machineId), [live.sessions, machineId]);
    const visible = useMemo(() => {
        const q = query.trim().toLowerCase();

        return onMachine.filter(
            (s) =>
                FILTERS[filter].test(s) &&
                (!q || [s.title, s.project, s.machine, agentName(s.agent), s.branch ?? ''].some((v) => v.toLowerCase().includes(q))),
        );
    }, [onMachine, filter, query]);
    const current = live.sessions.find((s) => s.id === selected) ?? visible[0] ?? onMachine[0] ?? null;
    const online = (s: RemoteSession) => live.machines.find((m) => m.id === s.machine_id)?.online ?? false;

    const open = (s: RemoteSession) => {
        setSelected(s.id);
        setView('focus');
    };
    const [newSession, setNewSession] = useState<{ machineId: number; folderId?: number } | 'open' | null>(null);
    const startNew = (at?: { machineId: number; folderId?: number }) => setNewSession(at ?? 'open');

    const shared: Shared = { live, sessions: onMachine, machines: live.machines, machineId, setMachineId, open, online, startNew };
    const dialog = (
        <NewSessionDialog
            open={newSession !== null}
            onClose={() => setNewSession(null)}
            initial={newSession && newSession !== 'open' ? newSession : machineId !== 'all' ? { machineId } : undefined}
            onStarted={(id) => {
                setSelected(id);
                setView('focus');
            }}
        />
    );

    const focus = current && (
        <SessionFocus
            key={current.id}
            session={current}
            online={online(current)}
            lastSeen={live.machines.find((m) => m.id === current.machine_id)?.last_seen_at}
            pending={live.pending[current.id]}
            onAnswer={(choice) => live.answer(current, choice)}
            onAction={(action, body, label) => live.act(current, action, body, label)}
            onBack={narrow ? () => setView('grid') : undefined}
        />
    );

    if (narrow) {
        return (
            <>
                {view === 'focus' && focus ? <div className="flex min-h-dvh flex-col bg-bg text-sm text-ink">{focus}</div> : <PhoneLayout {...shared} />}
                {dialog}
            </>
        );
    }

    return (
        <>
        {dialog}
        <DesktopLayout
            {...shared}
            visible={visible}
            filter={filter}
            setFilter={setFilter}
            query={query}
            setQuery={setQuery}
            view={view}
            setView={setView}
            current={current}
            focus={focus}
        />
        </>
    );
}

type Shared = {
    live: ReturnType<typeof useLive>;
    /** The sessions on the chosen machine(s), before the filter. */
    sessions: RemoteSession[];
    machines: Machine[];
    machineId: number | 'all';
    setMachineId: (id: number | 'all') => void;
    open: (s: RemoteSession) => void;
    online: (s: RemoteSession) => boolean;
    /** Opens the New session dialog, optionally on a machine's project. */
    startNew: (at?: { machineId: number; folderId?: number }) => void;
};

/** Which machine's sessions to show: a header chip. */
function MachineSelect({ machines, machineId, setMachineId }: Pick<Shared, 'machines' | 'machineId' | 'setMachineId'>) {
    const selectedMachine = machines.find((m) => m.id === machineId);
    const on = selectedMachine ? selectedMachine.online : machines.some((m) => m.online);

    return (
        <label className="hchip relative gap-2 pr-1.5 focus-within:border-outline-strong">
            <span className="sr-only">Machine</span>
            <span aria-hidden="true" className="dot" style={{ background: on ? '#3FB950' : '#8B929C' }} />
            <select
                value={machineId}
                onChange={(e) => setMachineId(e.target.value === 'all' ? 'all' : Number(e.target.value))}
                className="max-w-[180px] cursor-pointer appearance-none bg-transparent pr-5 text-ink-soft outline-none"
            >
                <option value="all">All machines</option>
                {machines.map((m) => (
                    <option key={m.id} value={m.id}>
                        {m.name}
                        {m.online ? '' : ' (offline)'}
                    </option>
                ))}
            </select>
            <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className="pointer-events-none absolute right-2.5 text-faint">
                <path d="m6 9 6 6 6-6" />
            </svg>
        </label>
    );
}

function PlusIcon() {
    return (
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
            <path d="M12 5v14M5 12h14" />
        </svg>
    );
}

function FolderIcon({ size = 15 }: { size?: number }) {
    return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d="M3 6h6l2 2h10v11H3z" />
        </svg>
    );
}

function EmptyState({ machines, filtered }: { machines: Machine[]; filtered: boolean }) {
    return (
        <div className="empty">
            <h2>{machines.length === 0 ? 'No machines paired yet' : filtered ? 'Nothing matches' : 'No sessions here'}</h2>
            <p>
                {machines.length === 0 ? (
                    <>
                        Pair Termlab on the <Link to="/settings/machines">Machines</Link> settings, then its sessions show up here.
                    </>
                ) : filtered ? (
                    'Try another filter, or clear the search.'
                ) : (
                    'Sessions that Termlab runs on your machines appear here as soon as they start.'
                )}
            </p>
        </div>
    );
}

/** "1 running · 1 need attention", as under a project's name. */
function summary(list: RemoteSession[]): string {
    const n = (st: SessionStatus) => list.filter((s) => s.status === st).length;
    const parts: string[] = [];
    if (n('running')) parts.push(`${n('running')} running`);
    if (n('needs_you') + n('error')) parts.push(`${n('needs_you') + n('error')} need attention`);
    if (n('idle')) parts.push(`${n('idle')} idle`);
    if (n('done')) parts.push(`${n('done')} done`);

    return parts.join(' · ') || 'no sessions';
}

/** Grouped by project in Termlab's sidebar order, newest session first, as in the app. */
function byProject(list: RemoteSession[]): [string, RemoteSession[]][] {
    const groups = new Map<string, RemoteSession[]>();
    for (const s of list) {
        groups.set(s.project, [...(groups.get(s.project) ?? []), s]);
    }
    const order = (l: RemoteSession[]) => Math.min(...l.map((s) => s.project_order ?? Number.MAX_SAFE_INTEGER));

    return [...groups.entries()]
        .map(([project, l]): [string, RemoteSession[]] => [project, [...l].sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0))])
        .sort(([, a], [, b]) => order(a) - order(b));
}

function waitingText(s: RemoteSession): string {
    if (s.status === 'error') {
        return 'exited with an error';
    }
    if (s.approval?.kind === 'question') {
        return 'has a question';
    }

    return s.approval?.detail ?? 'needs your input';
}

function DesktopLayout({
    live,
    sessions,
    machines,
    machineId,
    setMachineId,
    open,
    online,
    startNew,
    visible,
    filter,
    setFilter,
    query,
    setQuery,
    view,
    setView,
    current,
    focus,
}: Shared & {
    visible: RemoteSession[];
    filter: Filter;
    setFilter: (f: Filter) => void;
    query: string;
    setQuery: (q: string) => void;
    view: View;
    setView: (v: View) => void;
    current: RemoteSession | null;
    focus: ReactNode;
}) {
    const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
    const running = sessions.filter((s) => !s.on_hold && s.status === 'running').length;
    const waiting = sessions.filter(needsAttention);
    const filtering = filter !== 'all' || query.trim() !== '';
    // The sidebar lists every project (empty ones too), unless filtering; Mission control only
    // the ones with sessions.
    const groups = projectGroups(visible, machines, live.projects, machineId, !filtering);
    const tiles = groups.filter((g) => g.list.length > 0);
    const [drag, setDrag] = useState<{ machineId: number; folderId: number } | null>(null);
    const [drop, setDrop] = useState<{ key: string; after: boolean } | null>(null);
    const canStart = machines.some((m) => m.online && (live.projects[m.id]?.projects.length ?? 0) > 0);

    /** Moves a project before another of the same machine (null: last), in Termlab too. */
    const moveTo = (g: Group, before: number | null) => {
        if (g.folderId === null || before === g.folderId) return;
        void live.moveProject(g.machineId, g.folderId, before);
    };
    const siblings = (g: Group) => groups.filter((x) => x.machineId === g.machineId && x.folderId !== null);
    const manyMachines = machines.length > 1 && machineId === 'all';
    const counts = Object.fromEntries((Object.keys(FILTERS) as Filter[]).map((f) => [f, sessions.filter(FILTERS[f].test).length])) as Record<Filter, number>;

    const toggle = (project: string) =>
        setCollapsed((c) => {
            const next = new Set(c);
            if (next.has(project)) {
                next.delete(project);
            } else {
                next.add(project);
            }

            return next;
        });

    return (
        <div className="flex h-dvh flex-col overflow-hidden bg-bg text-sm leading-normal text-ink">
            <TopBar>
                <Segmented
                    label="View"
                    value={view}
                    onChange={setView}
                    options={[
                        { value: 'focus', label: 'Focus' },
                        { value: 'grid', label: 'Mission control' },
                    ]}
                />
                <div className="flex flex-wrap items-center gap-2">
                    <span className="hchip">
                        <span aria-hidden="true" className="dot" style={{ background: STATUS.running.color }} />
                        {running} running
                    </span>
                    {waiting.length > 0 && (
                        <button type="button" className="hchip amber" onClick={() => open(waiting[0])}>
                            ▲ {waiting.length} need{waiting.length === 1 ? 's' : ''} you
                        </button>
                    )}
                    <MachineSelect machines={machines} machineId={machineId} setMachineId={setMachineId} />
                    <UsageChip />
                </div>
            </TopBar>

            <div className="flex min-h-0 flex-1">
                <aside className="sidebar" aria-label="Sessions">
                    <div className="sidebar-top">
                        <label className="search-field">
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
                                <circle cx="11" cy="11" r="7" />
                                <path d="m20 20-3.5-3.5" />
                            </svg>
                            <span className="sr-only">Filter sessions</span>
                            <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Filter sessions" />
                        </label>
                        <div role="group" aria-label="Show" className="filter-tabs">
                            {(Object.keys(FILTERS) as Filter[]).map((key) => (
                                <button key={key} type="button" className={cn(filter === key && 'on')} aria-pressed={filter === key} onClick={() => setFilter(key)}>
                                    {FILTERS[key].label}
                                    {counts[key] > 0 && (
                                        <span aria-hidden="true" className={cn('filter-count', key === 'attention' && 'hot')}>
                                            {counts[key]}
                                        </span>
                                    )}
                                    <span className="sr-only">({counts[key]})</span>
                                </button>
                            ))}
                        </div>
                    </div>
                    <div className="sidebar-list">
                        <div className="list-head">
                            <span className="section-heading">Projects</span>
                            {canStart && (
                                <button type="button" className="icon-btn list-head-btn" aria-label="New session" title="New session" onClick={() => startNew()}>
                                    <PlusIcon />
                                </button>
                            )}
                        </div>
                        {groups.length === 0 && <p className="sidebar-note">{sessions.length === 0 ? 'No sessions yet.' : 'No sessions match.'}</p>}
                        {groups.map((g) => {
                            const { key, name: project, list } = g;
                            const isOpen = !collapsed.has(key);
                            const attention = list.filter(needsAttention).length;
                            const id = `project-${key.replace(/[^a-z0-9]+/gi, '-')}`;
                            const movable = g.folderId !== null && !filtering && (live.machines.find((m) => m.id === g.machineId)?.online ?? false);
                            const sameMachine = drag !== null && drag.machineId === g.machineId && g.folderId !== null;

                            return (
                                <div
                                    key={key}
                                    className={cn('folder-group', drag?.folderId === g.folderId && drag?.machineId === g.machineId && 'is-dragging', drop?.key === key && (drop.after ? 'drop-after' : 'drop-before'))}
                                    onDragOver={(e) => {
                                        if (!sameMachine) return;
                                        e.preventDefault();
                                        const r = e.currentTarget.getBoundingClientRect();
                                        setDrop({ key, after: e.clientY > r.top + r.height / 2 });
                                    }}
                                    onDragLeave={(e) => {
                                        if (!e.currentTarget.contains(e.relatedTarget as Node)) setDrop((d) => (d?.key === key ? null : d));
                                    }}
                                    onDrop={(e) => {
                                        e.preventDefault();
                                        const moving = drag && groups.find((x) => x.machineId === drag.machineId && x.folderId === drag.folderId);
                                        if (moving && sameMachine && drop) {
                                            const sib = siblings(g);
                                            const at = sib.findIndex((x) => x.key === key);
                                            const before = drop.after ? (sib[at + 1]?.folderId ?? null) : g.folderId;
                                            moveTo(moving, before);
                                        }
                                        setDrag(null);
                                        setDrop(null);
                                    }}
                                >
                                    <div className="folder-head">
                                        {movable && (
                                            <button
                                                type="button"
                                                className="folder-grip"
                                                draggable
                                                aria-label={`Move ${project}`}
                                                title="Drag to reorder (or Alt+↑/↓)"
                                                onDragStart={(e) => {
                                                    e.dataTransfer.effectAllowed = 'move';
                                                    e.dataTransfer.setData('text/plain', project);
                                                    setDrag({ machineId: g.machineId, folderId: g.folderId! });
                                                }}
                                                onDragEnd={() => {
                                                    setDrag(null);
                                                    setDrop(null);
                                                }}
                                                onKeyDown={(e) => {
                                                    if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
                                                    e.preventDefault();
                                                    const sib = siblings(g);
                                                    const at = sib.findIndex((x) => x.key === key);
                                                    if (e.key === 'ArrowUp' && at > 0) moveTo(g, sib[at - 1].folderId);
                                                    if (e.key === 'ArrowDown' && at < sib.length - 1) moveTo(g, sib[at + 2]?.folderId ?? null);
                                                }}
                                            >
                                                <svg width="10" height="14" viewBox="0 0 10 14" fill="currentColor" aria-hidden="true">
                                                    {[2, 7, 12].flatMap((y) => [<circle key={`a${y}`} cx="2.5" cy={y} r="1.3" />, <circle key={`b${y}`} cx="7.5" cy={y} r="1.3" />])}
                                                </svg>
                                            </button>
                                        )}
                                        <button type="button" className="folder-toggle" aria-expanded={isOpen} aria-controls={id} onClick={() => toggle(key)}>
                                            <span className={cn('chevron', isOpen && 'open')} aria-hidden="true">
                                                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                                    <path d="m9 6 6 6-6 6" />
                                                </svg>
                                            </span>
                                            <span className="folder-name">{project}</span>
                                            {manyMachines && <span className="truncate text-[11px] font-normal text-faint">{live.machines.find((m) => m.id === g.machineId)?.name}</span>}
                                            {attention > 0 && (
                                                <span className="folder-attention" title={`${attention} need${attention === 1 ? 's' : ''} attention`}>
                                                    {attention}
                                                </span>
                                            )}
                                            <span className="folder-count">{list.length}</span>
                                        </button>
                                        {g.folderId !== null && canStart && (live.machines.find((m) => m.id === g.machineId)?.online ?? false) && (
                                            <button
                                                type="button"
                                                className="icon-btn folder-add"
                                                aria-label={`New session in ${project}`}
                                                title={`New session in ${project}`}
                                                onClick={() => startNew({ machineId: g.machineId, folderId: g.folderId! })}
                                            >
                                                <PlusIcon />
                                            </button>
                                        )}
                                    </div>
                                    {isOpen && list.length > 0 && (
                                        <div id={id} className="folder-sessions">
                                            {list.map((s) => {
                                                const isCur = view === 'focus' && current?.id === s.id;

                                                return (
                                                    <button
                                                        key={s.id}
                                                        type="button"
                                                        onClick={() => open(s)}
                                                        aria-current={isCur ? 'true' : undefined}
                                                        className={cn('session-row', isCur && 'is-selected')}
                                                    >
                                                        <Monogram agent={s.agent} size={24} />
                                                        <span className="row-text">
                                                            <span className="row-title" title={s.title}>
                                                                {s.title}
                                                            </span>
                                                            <span className="row-meta">
                                                                <StatusDot status={s.status} />
                                                                <StatusText status={s.status} />
                                                                {manyMachines && <span className="truncate text-faint">· {s.machine}</span>}
                                                            </span>
                                                        </span>
                                                        <span className="row-time">{relativeTime(lastActivity(s))}</span>
                                                    </button>
                                                );
                                            })}
                                        </div>
                                    )}
                                </div>
                            );
                        })}
                    </div>
                </aside>

                <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-bg">
                    {view === 'focus' && focus ? (
                        focus
                    ) : (
                        <div className="mission">
                            {waiting.length > 0 && (
                                <section aria-label="Needs you" className="waiting-banner">
                                    <span className="waiting-title">
                                        ▲ {waiting.length} session{waiting.length === 1 ? ' is' : 's are'} waiting on you
                                    </span>
                                    {waiting.slice(0, 3).map((s) => (
                                        <span key={s.id} className="waiting-item">
                                            <b>{s.project}</b> · {agentName(s.agent)} {waitingText(s)}
                                        </span>
                                    ))}
                                    <div className="flex-1" />
                                    <Button variant="amber" onClick={() => open(waiting[0])}>
                                        Review one by one
                                    </Button>
                                </section>
                            )}

                            {visible.length === 0 && <EmptyState machines={machines} filtered={sessions.length > 0} />}

                            {tiles.map(({ key, name: project, list }) => {
                                const hosts = [...new Set(list.map((s) => s.machine))];

                                return (
                                    <section key={key} className="mc-group" aria-label={project}>
                                        <div className="mc-group-head">
                                            <FolderIcon />
                                            <h2>{project}</h2>
                                            {manyMachines && <span className="font-mono text-xs text-faint">{hosts.join(', ')}</span>}
                                            <span className="text-xs text-dim">· {summary(list)}</span>
                                        </div>
                                        <div className="tiles">
                                            {list.map((s) => (
                                                <Tile key={s.id} session={s} online={online(s)} showMachine={manyMachines} live={live} open={open} />
                                            ))}
                                        </div>
                                    </section>
                                );
                            })}
                        </div>
                    )}
                </main>
            </div>
        </div>
    );
}

function Tile({
    session: s,
    online,
    showMachine,
    live,
    open,
}: {
    session: RemoteSession;
    online: boolean;
    showMachine: boolean;
    live: Shared['live'];
    open: (s: RemoteSession) => void;
}) {
    const hot = needsAttention(s);
    // As in Focus: an approval Termlab couldn't read the options of still takes an answer.
    const canAllow = s.status === 'needs_you' && !!s.approval && (s.approval.options.length === 0 || s.approval.options.some((o) => o.choice === 'allow_once'));
    const lines = s.preview.filter((line) => line?.trim()).slice(-5);
    const pending = live.pending[s.id];

    return (
        <article className="tile" style={{ borderColor: hot ? STATUS[s.status].color + '88' : undefined }}>
            <div className="tile-head">
                <Monogram agent={s.agent} size={32} />
                <button type="button" className="tile-title" onClick={() => open(s)}>
                    <h3 title={s.title}>{s.title}</h3>
                    <span className="truncate text-xs text-dim">
                        {agentName(s.agent)}
                        {s.model ? <span className="font-mono"> · {s.model}</span> : null}
                    </span>
                </button>
                <StatusPill status={s.status} compact />
            </div>
            <div className="tile-preview" aria-label="Last terminal lines">
                {(lines.length ? lines : [' ']).map((line, i) => (
                    <div key={i}>{line || ' '}</div>
                ))}
            </div>
            <div className="tile-foot">
                <span>ctx {s.context_pct ?? '—'}%</span>
                <span>{relativeTime(lastActivity(s))}</span>
                {showMachine && <span className="truncate">{s.machine}</span>}
                {!online && <span className="text-faint">offline</span>}
                <div className="flex-1" />
                {pending ? (
                    <span role="status">{pending}</span>
                ) : (
                    canAllow &&
                    online && (
                        <Button variant="primary" size="sm" onClick={() => live.answer(s, 'allow_once')}>
                            Allow
                        </Button>
                    )
                )}
                <Button variant="default" size="sm" onClick={() => open(s)}>
                    Open
                </Button>
            </div>
        </article>
    );
}

function PhoneLayout({ live, sessions, machines, machineId, setMachineId, open, online, startNew }: Shared) {
    const [tab, setTab] = useState<'queue' | 'all'>('queue');
    const queue = sessions.filter((s) => !s.on_hold && s.status === 'needs_you');
    const canStart = machines.some((m) => m.online && (live.projects[m.id]?.projects.length ?? 0) > 0);

    return (
        <div className="flex h-dvh flex-col overflow-hidden bg-bg text-sm leading-normal text-ink">
            <TopBar>
                <div className="ml-auto flex min-w-0 items-center gap-2">
                    <UsageChip />
                    {canStart && (
                        <button type="button" className="icon-btn size-11" aria-label="New session" title="New session" onClick={() => startNew()}>
                            <PlusIcon />
                        </button>
                    )}
                    {machines.length > 1 && <MachineSelect machines={machines} machineId={machineId} setMachineId={setMachineId} />}
                </div>
            </TopBar>
            <div className="px-4 pt-3 pb-2">
                <Segmented
                    label="Show"
                    stretch
                    value={tab}
                    onChange={setTab}
                    options={[
                        { value: 'queue', label: `Needs you · ${queue.length}` },
                        { value: 'all', label: 'All sessions' },
                    ]}
                />
            </div>

            <main className="flex flex-1 flex-col gap-3 overflow-y-auto px-4 pt-1 pb-4">
                {tab === 'queue' &&
                    queue.map((s) => (
                        <article key={s.id} className="flex flex-col gap-3 rounded-xl border border-amber-line bg-amber-panel p-4">
                            <div className="flex items-center gap-2.5">
                                <Monogram agent={s.agent} size={28} />
                                <span className="flex min-w-0 flex-1 flex-col">
                                    <span className="truncate text-[13px] font-semibold">{s.title}</span>
                                    <span className="truncate text-xs text-dim">
                                        {s.project} · {agentName(s.agent)}
                                    </span>
                                </span>
                                <span className="text-[11px] text-faint">{relativeTime(lastActivity(s))}</span>
                            </div>
                            <span className="font-semibold text-amber-soft">{s.approval?.question ?? 'Waiting for your input'}</span>
                            {s.approval?.detail && (
                                <code className="rounded-[8px] border border-selected bg-term p-3 font-mono text-[12.5px] break-all whitespace-pre-wrap text-ink">{s.approval.detail}</code>
                            )}
                            {s.approval && (
                                <ApprovalActions layout="stack" pending={online(s) ? live.pending[s.id] : `${s.machine} is offline`} onAnswer={(c) => live.answer(s, c)} />
                            )}
                            <Button variant="ghost" onClick={() => open(s)}>
                                Open terminal
                            </Button>
                        </article>
                    ))}

                {tab === 'queue' && queue.length === 0 && (
                    <div className="flex flex-col items-center gap-1.5 rounded-xl border border-dashed border-line-strong px-5 py-8 text-center">
                        <svg width="28" height="28" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="#3FB950" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <path d="M5 12.5l4.5 4.5L19 7.5" />
                        </svg>
                        <span className="font-semibold">Nothing waiting on you</span>
                        <span className="text-[13px] text-dim">We will notify you when a session needs you.</span>
                        <PushToggle compact className="mt-2" />
                    </div>
                )}

                {tab === 'all' &&
                    (sessions.length === 0 ? (
                        <EmptyState machines={machines} filtered={false} />
                    ) : (
                        <div className="flex flex-col gap-1">
                            {projectGroups(sessions, machines, live.projects, machineId, false).map(({ key, name: project, list }) => (
                                <section key={key} aria-label={project} className="flex flex-col gap-px">
                                    <div className="flex items-center gap-2 px-2 pt-2 pb-1">
                                        <span className="section-heading">{project}</span>
                                        <span className="text-xs text-faint">{list.length}</span>
                                    </div>
                                    {list.map((s) => (
                                        <button key={s.id} type="button" onClick={() => open(s)} className="session-row min-h-[52px]">
                                            <Monogram agent={s.agent} size={28} />
                                            <span className="row-text">
                                                <span className="row-title text-sm">{s.title}</span>
                                                <span className="row-meta">
                                                    <StatusDot status={s.status} />
                                                    <StatusText status={s.status} />
                                                    {s.on_hold && <span className="text-faint">· On hold</span>}
                                                    {!online(s) && <span className="text-faint">· {s.machine} offline</span>}
                                                </span>
                                            </span>
                                            <span className="row-time">{relativeTime(lastActivity(s)) || timeAgo(s.updated_at)}</span>
                                        </button>
                                    ))}
                                </section>
                            ))}
                        </div>
                    ))}
            </main>

            <BottomNav />
        </div>
    );
}
