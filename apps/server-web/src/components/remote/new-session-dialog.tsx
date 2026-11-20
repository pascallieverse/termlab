import { useEffect, useId, useState } from 'react';
import { AgentLogo } from '@/components/el/agent';
import { Button } from '@/components/el/button';
import { Dialog } from '@/components/el/dialog';
import { agentId } from '@/lib/format';
import { useLive } from '@/live/LiveProvider';
import type { PermissionPreset } from '@/types';

/** As in the desktop app (apps/desktop/src/agents.ts). */
const PERMISSIONS: { key: PermissionPreset; label: string; desc: string }[] = [
    { key: 'ask_first', label: 'Ask first', desc: 'Approve every edit and command' },
    { key: 'auto_edit', label: 'Auto-edit', desc: 'Edits OK, ask before commands' },
    { key: 'full_auto', label: 'Full auto', desc: 'No prompts. Use in a sandbox' },
];

const LAST_KEY = 'termlab.newSession';

type Last = { agent?: string; permission?: PermissionPreset };

function last(): Last {
    try {
        return JSON.parse(localStorage.getItem(LAST_KEY) ?? '{}') as Last;
    } catch {
        return {};
    }
}

/**
 * A new session on one of your computers, as Termlab's New session dialog: a project, an
 * agent installed there, how much it may do on its own, and optionally a first message.
 * Termlab starts it, so it shows up in the app at once as well.
 */
export function NewSessionDialog({
    open,
    onClose,
    initial,
    onStarted,
}: {
    open: boolean;
    onClose: () => void;
    /** Preselected machine and project (from a project's + button). */
    initial?: { machineId: number; folderId?: number };
    onStarted: (sessionId: string) => void;
}) {
    const live = useLive();
    const ids = useId();
    // Machines that can start sessions: online, with a Termlab that reports its projects.
    const machines = live.machines.filter((m) => m.online && (live.projects[m.id]?.projects.length ?? 0) > 0);
    const [machineId, setMachineId] = useState<number | null>(null);
    const [folderId, setFolderId] = useState<number | null>(null);
    const [agent, setAgent] = useState<string | null>(null);
    const [permission, setPermission] = useState<PermissionPreset>('ask_first');
    const [model, setModel] = useState('');
    const [message, setMessage] = useState('');
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    // Fresh choices each time it opens.
    useEffect(() => {
        if (!open) return;
        const mid = initial?.machineId ?? machines[0]?.id ?? null;
        const mp = mid !== null ? live.projects[mid] : undefined;
        const remembered = last();
        setMachineId(mid);
        setFolderId(initial?.folderId ?? mp?.projects[0]?.id ?? null);
        setAgent(mp?.agents.find((a) => a.agent === remembered.agent)?.agent ?? mp?.agents[0]?.agent ?? null);
        setPermission(remembered.permission ?? 'ask_first');
        setModel('');
        setMessage('');
        setError(null);
        setBusy(false);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open]);

    const mp = machineId !== null ? live.projects[machineId] : undefined;
    const agentInfo = mp?.agents.find((a) => a.agent === agent);
    const machine = live.machines.find((m) => m.id === machineId);

    async function start(e: React.FormEvent) {
        e.preventDefault();
        if (machineId === null || folderId === null || !agent) return;
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
                model: agentInfo?.model_flag && model.trim() ? model.trim() : null,
                message: message.trim() ? message : null,
            });
            onStarted(id);
            onClose();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
            setBusy(false);
        }
    }

    return (
        <Dialog open={open} onClose={onClose} title="New session" description={machine ? `Starts on ${machine.name}, in Termlab.` : undefined}>
            {machines.length === 0 ? (
                <p className="m-0 text-[13px] leading-relaxed text-dim">
                    Starting sessions from here needs a computer that's online and runs Termlab 0.2.9 or newer.
                </p>
            ) : (
                <form onSubmit={start} className="new-session">
                    {machines.length > 1 && (
                        <label className="ns-field">
                            <span className="ns-label">Computer</span>
                            <select
                                className="ns-input"
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
                        </label>
                    )}

                    <label className="ns-field">
                        <span className="ns-label">Project</span>
                        <select className="ns-input" value={folderId ?? ''} onChange={(e) => setFolderId(Number(e.target.value))}>
                            {mp?.projects.map((p) => (
                                <option key={p.id} value={p.id}>
                                    {p.name}
                                </option>
                            ))}
                        </select>
                    </label>

                    <fieldset className="ns-field">
                        <legend className="ns-label">Agent</legend>
                        {mp && mp.agents.length === 0 && <p className="m-0 text-[13px] text-dim">No agents are installed on {machine?.name}.</p>}
                        <div className="ns-cards" role="radiogroup" aria-label="Agent">
                            {mp?.agents.map((a) => (
                                <button
                                    key={a.agent}
                                    type="button"
                                    role="radio"
                                    aria-checked={agent === a.agent}
                                    className={`ns-card${agent === a.agent ? ' on' : ''}`}
                                    onClick={() => setAgent(a.agent)}
                                >
                                    <AgentLogo agent={agentId(a.agent)} size={18} />
                                    <span className="font-medium">{a.name}</span>
                                </button>
                            ))}
                        </div>
                    </fieldset>

                    <fieldset className="ns-field">
                        <legend className="ns-label">Permissions</legend>
                        <div className="ns-cards" role="radiogroup" aria-label="Permissions">
                            {PERMISSIONS.map((p) => (
                                <button
                                    key={p.key}
                                    type="button"
                                    role="radio"
                                    aria-checked={permission === p.key}
                                    className={`ns-card${permission === p.key ? ' on' : ''}`}
                                    onClick={() => setPermission(p.key)}
                                >
                                    <span className="font-medium">{p.label}</span>
                                    <span className="text-xs text-dim">{p.desc}</span>
                                </button>
                            ))}
                        </div>
                    </fieldset>

                    {agentInfo?.model_flag && (
                        <label className="ns-field">
                            <span className="ns-label">Model</span>
                            <input
                                className="ns-input"
                                value={model}
                                onChange={(e) => setModel(e.target.value)}
                                placeholder="The agent's default"
                                list={`${ids}-models`}
                                autoComplete="off"
                            />
                            <datalist id={`${ids}-models`}>
                                {agentInfo.models.map((m) => (
                                    <option key={m} value={m} />
                                ))}
                            </datalist>
                        </label>
                    )}

                    <label className="ns-field">
                        <span className="ns-label">First message (optional)</span>
                        <textarea
                            className="ns-input ns-message"
                            value={message}
                            onChange={(e) => setMessage(e.target.value)}
                            placeholder="What should it work on?"
                            rows={3}
                            onKeyDown={(e) => {
                                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                                    e.currentTarget.form?.requestSubmit();
                                }
                            }}
                        />
                    </label>

                    {error && (
                        <p role="alert" className="m-0 text-[13px] text-bad">
                            {error}
                        </p>
                    )}

                    <div className="flex justify-end gap-2">
                        <Button variant="ghost" onClick={onClose}>
                            Cancel
                        </Button>
                        <Button type="submit" variant="primary" disabled={busy || folderId === null || !agent}>
                            {busy ? 'Starting…' : 'Start session'}
                        </Button>
                    </div>
                </form>
            )}
        </Dialog>
    );
}
