import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { Monogram } from '@/components/el/agent';
import { Dialog } from '@/components/el/dialog';
import { atLeast, relativeTime } from '@/lib/format';
import { useLive } from '@/live/LiveProvider';
import type { PermissionPreset } from '@/types';

// Termlab's New session dialog (apps/desktop/src/NewSessionDialog.tsx and FolderPicker.tsx),
// for one of your computers: Termlab starts the session, so the app shows it at once too.

/** As in the desktop app (apps/desktop/src/agents.ts). */
const PERMISSIONS: { key: PermissionPreset; label: string; desc: string }[] = [
    { key: 'ask_first', label: 'Ask first', desc: 'Approve every edit and command' },
    { key: 'auto_edit', label: 'Auto-edit', desc: 'Edits OK, ask before commands' },
    { key: 'full_auto', label: 'Full auto', desc: 'No prompts. Use in a sandbox' },
];

/** The command each agent runs, under its name on the cards (the desktop shows the same). */
const BINARY: Record<string, string> = { claude: 'claude', codex: 'codex', vibe: 'vibe', antigravity: 'agy', shell: 'shell' };

const CUSTOM = '__custom__';
/** The folder list gets a filter from this many folders, as in the app. */
const FILTER_FROM = 6;
const LAST_KEY = 'termlab.newSession';
const MOD = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';

type Last = { agent?: string; permission?: PermissionPreset };

function last(): Last {
    try {
        return JSON.parse(localStorage.getItem(LAST_KEY) ?? '{}') as Last;
    } catch {
        return {};
    }
}

/** "/Users/pascal/Code/api" → "~/Code/api". */
function tildify(path: string): string {
    return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, '~');
}

export function NewSessionDialog({
    open,
    onClose,
    initial,
    onStarted,
}: {
    open: boolean;
    onClose: () => void;
    /** Preselected machine and project (from a project's +, which fixes the project). */
    initial?: { machineId: number; folderId?: number };
    onStarted: (sessionId: string) => void;
}) {
    const live = useLive();
    // Computers that can start sessions: online, with a Termlab that reports its projects.
    const machines = live.machines.filter((m) => m.online && (live.projects[m.id]?.projects.length ?? 0) > 0);
    const [machineId, setMachineId] = useState<number | null>(null);
    const [folderId, setFolderId] = useState<number | null>(null);
    const [agent, setAgent] = useState<string | null>(null);
    const [permission, setPermission] = useState<PermissionPreset>('ask_first');
    const [modelChoice, setModelChoice] = useState('');
    const [customModel, setCustomModel] = useState('');
    const [notify, setNotify] = useState(true);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const fixedFolder = initial?.folderId !== undefined;

    // Fresh choices each time it opens.
    useEffect(() => {
        if (!open) return;
        const mid = initial?.machineId ?? machines[0]?.id ?? null;
        const mp = mid !== null ? live.projects[mid] : undefined;
        const remembered = last();
        // As in the app: the folder of a session that ran most recently, else the first.
        const recent = [...live.sessions]
            .filter((s) => s.machine_id === mid && s.folder_id != null)
            .sort((a, b) => (b.updated_at > a.updated_at ? 1 : -1))[0]?.folder_id;
        setMachineId(mid);
        setFolderId(initial?.folderId ?? mp?.projects.find((p) => p.id === recent)?.id ?? mp?.projects[0]?.id ?? null);
        setAgent(mp?.agents.find((a) => a.agent === remembered.agent)?.agent ?? mp?.agents[0]?.agent ?? null);
        setPermission(remembered.permission ?? 'ask_first');
        setModelChoice('');
        setCustomModel('');
        setNotify(true);
        setError(null);
        setBusy(false);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open]);

    useEffect(() => {
        setModelChoice('');
        setCustomModel('');
    }, [agent]);

    const mp = machineId !== null ? live.projects[machineId] : undefined;
    const info = mp?.agents.find((a) => a.agent === agent);
    const machine = live.machines.find((m) => m.id === machineId);
    const shell = agent === 'shell';
    const model = modelChoice === CUSTOM ? customModel.trim() : modelChoice;
    const fixed = fixedFolder ? mp?.projects.find((p) => p.id === folderId) : undefined;
    // Termlab 0.2.10 and newer take the notify choice; before, sessions always notify.
    const canNotify = atLeast(machine?.version, '0.2.10');
    const ready = machineId !== null && folderId !== null && !!agent && !busy;

    async function start() {
        if (!ready || machineId === null || folderId === null || !agent) return;
        setBusy(true);
        setError(null);
        try {
            localStorage.setItem(LAST_KEY, JSON.stringify({ agent, permission }));
        } catch {
            // Only a convenience.
        }
        try {
            const id = await live.startSession(machineId, {
                folder_id: folderId,
                agent,
                permission,
                model: info?.model_flag && model ? model : null,
                notify: notify && !shell,
            });
            onStarted(id);
            onClose();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setBusy(false);
        }
    }

    return (
        <Dialog
            open={open}
            onClose={onClose}
            width={720}
            title="New session"
            head={
                <>
                    {fixed && (
                        <span className="truncate font-mono text-xs text-faint" title={fixed.path}>
                            {tildify(fixed.path)}
                        </span>
                    )}
                    {machine && machines.length > 1 && <span className="text-xs text-faint">on {machine.name}</span>}
                </>
            }
            footer={
                machines.length > 0 && (
                    <>
                        {!shell && canNotify && (
                            <label className="ns-check">
                                <input type="checkbox" checked={notify} onChange={(e) => setNotify(e.target.checked)} />
                                Notify me when it needs input
                            </label>
                        )}
                        <div className="flex-1" />
                        <button type="button" className="ns-btn" onClick={onClose}>
                            Cancel
                        </button>
                        <button type="button" className="ns-btn primary" disabled={!ready} onClick={() => void start()}>
                            {busy ? 'Starting…' : shell ? 'Open terminal' : 'Start session'}
                            <span className="ns-kbd">{MOD}↵</span>
                        </button>
                    </>
                )
            }
        >
            {machines.length === 0 ? (
                <p className="m-0 text-[13px] leading-relaxed text-dim">Starting sessions from here needs a computer that's online and runs Termlab 0.2.9 or newer.</p>
            ) : (
                <div
                    className="new-session"
                    onKeyDown={(e) => {
                        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                            e.preventDefault();
                            void start();
                        }
                    }}
                >
                    {machines.length > 1 && (
                        <div className="ns-field">
                            <label htmlFor="ns-machine" className="ns-label">
                                Computer
                            </label>
                            <select
                                id="ns-machine"
                                className="ns-select"
                                value={machineId ?? ''}
                                onChange={(e) => {
                                    const id = Number(e.target.value);
                                    const next = live.projects[id];
                                    setMachineId(id);
                                    setFolderId(next?.projects[0]?.id ?? null);
                                    setAgent(next?.agents.find((a) => a.agent === agent)?.agent ?? next?.agents[0]?.agent ?? null);
                                }}
                            >
                                {machines.map((m) => (
                                    <option key={m.id} value={m.id}>
                                        {m.name}
                                    </option>
                                ))}
                            </select>
                        </div>
                    )}

                    <fieldset className="ns-field">
                        <legend className="ns-label">Agent or terminal</legend>
                        <div className="ns-agent-cards">
                            {mp?.agents.map((a) => (
                                <button
                                    key={a.agent}
                                    type="button"
                                    className={`ns-agent-card${agent === a.agent ? ' on' : ''}`}
                                    aria-pressed={agent === a.agent}
                                    onClick={() => setAgent(a.agent)}
                                >
                                    <Monogram agent={a.agent} size={32} />
                                    <span className="ns-agent-name">{a.name}</span>
                                    <span className="font-mono text-xs text-faint">{BINARY[a.agent] ?? a.agent}</span>
                                </button>
                            ))}
                        </div>
                        {mp && mp.agents.length === 0 && <p className="m-0 text-xs text-dim">No agents are installed on {machine?.name}.</p>}
                    </fieldset>

                    {!fixedFolder && machineId !== null && mp && (
                        <FolderPicker machineId={machineId} projects={mp.projects} value={folderId} onChange={setFolderId} />
                    )}

                    {!shell && (
                        <div className="ns-two-col">
                            <div className="ns-field">
                                <label htmlFor="ns-model" className="ns-label">
                                    Model
                                </label>
                                {info && !info.model_flag ? (
                                    <select id="ns-model" className="ns-select" disabled>
                                        <option>Set in {info.name}'s own config</option>
                                    </select>
                                ) : (
                                    <>
                                        <select id="ns-model" className="ns-select" value={modelChoice} onChange={(e) => setModelChoice(e.target.value)}>
                                            <option value="">Default (the agent's own setting)</option>
                                            {info?.models.map((m) => (
                                                <option key={m} value={m}>
                                                    {m}
                                                </option>
                                            ))}
                                            <option value={CUSTOM}>Other…</option>
                                        </select>
                                        {modelChoice === CUSTOM && (
                                            <input
                                                className="ns-input font-mono"
                                                aria-label="Model name"
                                                placeholder="model name"
                                                value={customModel}
                                                onChange={(e) => setCustomModel(e.target.value)}
                                            />
                                        )}
                                    </>
                                )}
                            </div>
                            <div className="ns-field">
                                <label htmlFor="ns-iso" className="ns-label">
                                    Run in
                                </label>
                                <select id="ns-iso" className="ns-select" defaultValue="checkout">
                                    <option value="checkout">Current checkout</option>
                                </select>
                            </div>
                        </div>
                    )}

                    {!shell && (
                        <fieldset className="ns-field">
                            <legend className="ns-label">Permissions</legend>
                            <div className="ns-perm-cards">
                                {PERMISSIONS.map((p) => (
                                    <button
                                        key={p.key}
                                        type="button"
                                        className={`ns-perm-card${permission === p.key ? ' on' : ''}`}
                                        aria-pressed={permission === p.key}
                                        onClick={() => setPermission(p.key)}
                                    >
                                        <span className="ns-perm-label">{p.label}</span>
                                        <span className="text-xs text-dim">{p.desc}</span>
                                    </button>
                                ))}
                            </div>
                        </fieldset>
                    )}

                    {error && (
                        <p role="alert" className="m-0 text-[13px] text-bad">
                            {error}
                        </p>
                    )}
                </div>
            )}
        </Dialog>
    );
}

/** The app's folder picker: filter, then a list to choose from with what's open in each. */
function FolderPicker({
    machineId,
    projects,
    value,
    onChange,
}: {
    machineId: number;
    projects: { id: number; name: string; path: string }[];
    value: number | null;
    onChange: (id: number) => void;
}) {
    const live = useLive();
    const [query, setQuery] = useState('');
    const list = useRef<HTMLDivElement>(null);

    // What's happening in each folder.
    const stats = useMemo(() => {
        const m = new Map<number, { open: number; last: number }>();
        for (const s of live.sessions) {
            if (s.machine_id !== machineId || s.folder_id == null) continue;
            const cur = m.get(s.folder_id) ?? { open: 0, last: 0 };
            if (s.status !== 'done' && s.status !== 'error') cur.open += 1;
            cur.last = Math.max(cur.last, Date.parse(s.updated_at) || 0);
            m.set(s.folder_id, cur);
        }
        return m;
    }, [live.sessions, machineId]);

    // Most recently used first; fixed when the picker opens, so rows don't jump around.
    const [rank] = useState(() => {
        const sorted = [...projects].sort((a, b) => (stats.get(b.id)?.last ?? 0) - (stats.get(a.id)?.last ?? 0) || a.name.localeCompare(b.name));
        return new Map(sorted.map((f, i) => [f.id, i]));
    });
    const ordered = useMemo(() => [...projects].sort((a, b) => (rank.get(a.id) ?? -1) - (rank.get(b.id) ?? -1)), [projects, rank]);
    const q = query.trim().toLowerCase();
    const shown = q ? ordered.filter((f) => f.name.toLowerCase().includes(q) || f.path.toLowerCase().includes(q)) : ordered;

    useEffect(() => {
        list.current?.querySelector('[aria-checked="true"]')?.scrollIntoView({ block: 'nearest' });
    }, [value]);

    function focusRow(index: number) {
        list.current?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[index]?.focus();
    }

    function onRowKey(e: KeyboardEvent, index: number) {
        const step = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : 0;
        if (step === 0) return;
        e.preventDefault();
        const next = Math.min(shown.length - 1, Math.max(0, index + step));
        onChange(shown[next].id);
        focusRow(next);
    }

    return (
        <div className="ns-field">
            <span className="ns-label" id="ns-folder">
                Project folder
            </span>
            <div className="ns-folder-picker">
                {projects.length >= FILTER_FROM && (
                    <div className="ns-folder-filter">
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
                            <circle cx="11" cy="11" r="7" />
                            <path d="m20 20-3.5-3.5" />
                        </svg>
                        <label htmlFor="ns-folder-q" className="sr-only">
                            Filter folders
                        </label>
                        <input
                            id="ns-folder-q"
                            placeholder="Filter folders"
                            value={query}
                            onChange={(e) => setQuery(e.target.value)}
                            onKeyDown={(e) => {
                                if ((e.key === 'Enter' || e.key === 'ArrowDown') && shown.length > 0 && !(e.metaKey || e.ctrlKey)) {
                                    e.preventDefault();
                                    onChange(shown[0].id);
                                    if (e.key === 'ArrowDown') focusRow(0);
                                }
                            }}
                        />
                    </div>
                )}
                <div role="radiogroup" aria-labelledby="ns-folder" className="ns-folder-list" ref={list}>
                    {shown.map((f, i) => {
                        const st = stats.get(f.id);
                        const on = value === f.id;
                        return (
                            <button
                                type="button"
                                key={f.id}
                                role="radio"
                                aria-checked={on}
                                tabIndex={on || (value === null && i === 0) ? 0 : -1}
                                className={`ns-folder-row${on ? ' on' : ''}`}
                                onClick={() => onChange(f.id)}
                                onKeyDown={(e) => onRowKey(e, i)}
                            >
                                <span className="ns-folder-mark" aria-hidden="true" />
                                <span className="ns-folder-name">{f.name}</span>
                                <span className="ns-folder-path font-mono">{tildify(f.path)}</span>
                                <span className="ns-folder-meta">
                                    {st && st.open > 0 ? (
                                        <span className="text-info-dot">{st.open} open</span>
                                    ) : st && st.last > 0 ? (
                                        relativeTime(new Date(st.last).toISOString())
                                    ) : (
                                        'no sessions'
                                    )}
                                </span>
                            </button>
                        );
                    })}
                    {shown.length === 0 && <p className="m-0 p-2.5 text-xs text-dim">{projects.length === 0 ? 'No project folders yet.' : 'No folder matches.'}</p>}
                </div>
            </div>
        </div>
    );
}
