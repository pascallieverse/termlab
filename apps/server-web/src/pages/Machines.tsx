import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Button } from '@/components/el/button';
import { Dialog } from '@/components/el/dialog';
import { Panel } from '@/components/el/panel';
import { Pill } from '@/components/el/status';
import { useToast } from '@/components/el/toast';
import { HeadActions, SectionNote } from '@/layouts/SettingsLayout';
import { useLive } from '@/live/LiveProvider';
import { api, errorMessage } from '@/lib/api';
import { formatDate, timeAgo } from '@/lib/format';
import type { Machine } from '@/types';

/** Settings › Machines. */
export function MachinesSection() {
    const live = useLive();
    const [pairing, setPairing] = useState(false);
    const [managing, setManaging] = useState<Machine | null>(null);
    const machines = [...live.machines].sort((a, b) => a.name.localeCompare(b.name));
    const online = machines.filter((m) => m.online).length;

    return (
        <>
            <HeadActions>
                <Button onClick={() => setPairing(true)}>Pair a machine</Button>
            </HeadActions>
            {machines.length > 0 && (
                <SectionNote>
                    {online} of {machines.length} online.
                </SectionNote>
            )}

            <ServerAddress />

            <Panel title="Your machines" titleId="machines-heading">
                {machines.length === 0 ? (
                    <div className="flex flex-col items-start gap-3 px-5 py-6">
                        <p className="m-0 text-dim">
                            No machines yet. Pair Termlab on your computer to see its sessions here.
                        </p>
                        <Button variant="outline" onClick={() => setPairing(true)}>
                            Pair a machine
                        </Button>
                    </div>
                ) : (
                    <div className="relative overflow-x-auto">
                        <table className="w-full min-w-[560px] border-collapse">
                            <thead>
                                <tr className="text-left text-[13px] text-dim">
                                    <th scope="col" className="px-5 py-3 font-medium">Machine</th>
                                    <th scope="col" className="px-5 py-3 font-medium">Status</th>
                                    <th scope="col" className="px-5 py-3 font-medium">Sessions</th>
                                    <th scope="col" className="px-5 py-3 font-medium">Termlab</th>
                                    <th scope="col" className="px-5 py-3 font-medium">
                                        <span className="sr-only">Actions</span>
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {machines.map((m) => (
                                    <tr key={m.id} className="border-t border-line-soft">
                                        <th scope="row" className="px-5 py-3.5 text-left font-medium">
                                            <span className="flex flex-col">
                                                <span>{m.name}</span>
                                                <span className="text-[13px] font-normal text-dim">{m.os ?? '—'}</span>
                                            </span>
                                        </th>
                                        <td className="px-5 py-3.5">
                                            <span className="flex flex-col items-start gap-1">
                                                <Pill tone={m.online ? 'ok' : 'off'}>{m.online ? 'Online' : 'Offline'}</Pill>
                                                {!m.online && m.last_seen_at && (
                                                    <span className="text-xs text-dim">seen {timeAgo(m.last_seen_at)}</span>
                                                )}
                                            </span>
                                        </td>
                                        <td className="px-5 py-3.5 text-ink-soft">
                                            {m.sessions === 0 ? '—' : `${m.running} running · ${m.sessions} total`}
                                        </td>
                                        <td className="px-5 py-3.5 font-mono text-[13px] text-ink-soft">{m.version ?? '—'}</td>
                                        <td className="px-5 py-3.5 text-right">
                                            <Button variant="outline" size="sm" onClick={() => setManaging(m)}>
                                                Manage
                                            </Button>
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
            </Panel>

            <PairDialog open={pairing} onClose={() => setPairing(false)} />
            <ManageDialog machine={managing} onClose={() => setManaging(null)} />
        </>
    );
}

/** What to type into Termlab's "Server address" field. Clicking the address copies it. */
function ServerAddress() {
    const toast = useToast();
    const address = window.location.origin;
    const [copied, setCopied] = useState(false);

    useEffect(() => {
        if (!copied) return;
        const timer = window.setTimeout(() => setCopied(false), 2000);
        return () => window.clearTimeout(timer);
    }, [copied]);

    function copy() {
        navigator.clipboard.writeText(address).then(
            () => setCopied(true),
            () => toast.error('Could not copy. Select the address and copy it.'),
        );
    }

    return (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-[14px] border border-line bg-panel px-5 py-4">
            <div className="flex min-w-0 flex-col gap-0.5">
                <span className="text-[13px] text-dim">Server address for Termlab</span>
                <button
                    type="button"
                    onClick={copy}
                    title="Click to copy"
                    aria-label={`Copy ${address}`}
                    className={`group -mx-2 flex min-w-0 cursor-pointer items-center gap-2 rounded-lg border px-2 py-1 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink-soft ${
                        copied ? 'border-ok/40 bg-ok-bg' : 'border-transparent hover:border-line hover:bg-line-soft'
                    }`}
                >
                    <code className={`font-mono text-[15px] break-all transition-colors ${copied ? 'text-ok' : 'text-ink'}`}>{address}</code>
                    <span
                        aria-hidden="true"
                        className={`shrink-0 transition-opacity ${copied ? 'text-ok opacity-100' : 'text-dim opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100'}`}
                    >
                        {copied ? <CheckIcon /> : <CopyIcon />}
                    </span>
                </button>
            </div>
            <Button variant="outline" size="sm" onClick={copy}>
                <span className={`inline-flex items-center gap-1.5 ${copied ? 'text-ok' : ''}`}>
                    {copied ? <CheckIcon /> : <CopyIcon />}
                    {copied ? 'Copied' : 'Copy'}
                </span>
            </Button>
            <span role="status" className="sr-only">
                {copied ? 'Address copied' : ''}
            </span>
        </div>
    );
}

function CopyIcon() {
    return (
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
            <path d="M10.5 5.5V3.75A1.25 1.25 0 0 0 9.25 2.5h-5.5A1.25 1.25 0 0 0 2.5 3.75v5.5a1.25 1.25 0 0 0 1.25 1.25H5.5" />
        </svg>
    );
}

function CheckIcon() {
    return (
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round">
            <path d="M3 8.5l3.25 3.25L13 5" />
        </svg>
    );
}

type Code = { code: string; expires_at: string };

function useCountdown(until: string | null): number {
    const [now, setNow] = useState(() => Date.now());

    useEffect(() => {
        if (!until) {
            return;
        }
        const timer = window.setInterval(() => setNow(Date.now()), 1000);

        return () => window.clearInterval(timer);
    }, [until]);

    return until ? Math.max(0, Math.round((new Date(until).getTime() - now) / 1000)) : 0;
}

function PairDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
    const live = useLive();
    const toast = useToast();
    const [code, setCode] = useState<Code | null>(null);
    // The machines there were before this code: the one that pairs with it is any other.
    const known = useRef<Set<number> | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [loading, setLoading] = useState(false);
    const left = useCountdown(code?.expires_at ?? null);
    const expired = code !== null && left === 0;

    async function fetchCode() {
        setLoading(true);
        setError(null);
        known.current = new Set(live.machines.map((m) => m.id));
        try {
            setCode(await api.post<Code>('/api/pairing-codes'));
        } catch (e) {
            setCode(null);
            setError(errorMessage(e, 'Could not create a pairing code.'));
        } finally {
            setLoading(false);
        }
    }

    useEffect(() => {
        if (open) {
            void fetchCode();
        } else {
            setCode(null);
            setError(null);
            known.current = null;
        }
    }, [open]);

    // Closes by itself once the new machine connects.
    const linked = open && known.current ? live.machines.find((m) => m.online && !known.current!.has(m.id)) : undefined;
    useEffect(() => {
        if (!linked) return;
        toast.success(`${linked.name} is linked.`);
        onClose();
    }, [linked?.id]); // eslint-disable-line react-hooks/exhaustive-deps

    const minutes = Math.floor(left / 60);
    const seconds = String(left % 60).padStart(2, '0');

    return (
        <Dialog
            open={open}
            onClose={onClose}
            title="Pair a machine"
            description="Link a computer running Termlab to this server."
        >
            {error ? (
                <p role="alert" className="m-0 rounded-[10px] border border-bad-line bg-bad-bg p-3 text-sm text-bad">
                    {error}
                </p>
            ) : (
                <div className="flex flex-col items-center gap-2 rounded-xl border border-line bg-bg px-4 py-6">
                    <output
                        aria-live="polite"
                        className={
                            'font-mono text-[34px] font-medium tracking-[0.12em] ' +
                            (expired ? 'text-faint line-through' : 'text-amber')
                        }
                    >
                        {code?.code ?? (loading ? '····-····' : '')}
                    </output>
                    <span className="text-[13px] text-dim">
                        {code && !expired
                            ? `Expires in ${minutes}:${seconds}`
                            : expired
                              ? 'This code has expired.'
                              : 'Creating a code…'}
                    </span>
                </div>
            )}

            <ol className="m-0 flex list-decimal flex-col gap-1.5 pl-5 text-sm text-ink-soft">
                <li>In Termlab, open Settings › Remote access.</li>
                <li>
                    Enter this server’s address,{' '}
                    <code className="font-mono text-ink">{window.location.origin}</code>, and the code.
                </li>
                <li>This closes by itself as soon as the machine connects.</li>
            </ol>

            <div className="flex flex-wrap justify-end gap-2">
                {(expired || error) && (
                    <Button variant="outline" onClick={() => void fetchCode()} disabled={loading}>
                        New code
                    </Button>
                )}
                <Button onClick={onClose}>Done</Button>
            </div>
        </Dialog>
    );
}

function ManageDialog({ machine, onClose }: { machine: Machine | null; onClose: () => void }) {
    return (
        <Dialog
            open={machine !== null}
            onClose={onClose}
            title={machine?.name ?? ''}
            description={
                machine &&
                ([machine.os, machine.version && `Termlab ${machine.version}`].filter(Boolean).join(' · ') ||
                    'Paired machine') + (machine.last_seen_at ? ` · last seen ${formatDate(machine.last_seen_at)}` : '')
            }
        >
            {machine && <ManageMachine key={machine.id} machine={machine} onClose={onClose} />}
        </Dialog>
    );
}

function ManageMachine({ machine, onClose }: { machine: Machine; onClose: () => void }) {
    const live = useLive();
    const toast = useToast();
    const [name, setName] = useState(machine.name);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [confirming, setConfirming] = useState(false);

    async function save(e: FormEvent) {
        e.preventDefault();
        setBusy(true);
        setError(null);
        try {
            await api.patch(`/api/machines/${machine.id}`, { name: name.trim() });
            await live.reload();
            toast.success('Machine renamed.');
            onClose();
        } catch (err) {
            setError(errorMessage(err, 'Could not rename the machine.'));
        } finally {
            setBusy(false);
        }
    }

    async function remove() {
        setBusy(true);
        try {
            await api.del(`/api/machines/${machine.id}`);
            await live.reload();
            toast.success(`${machine.name} was removed.`);
            onClose();
        } catch (err) {
            setError(errorMessage(err, 'Could not remove the machine.'));
            setBusy(false);
        }
    }

    return (
        <>
            <form onSubmit={save} className="flex flex-col gap-2">
                <label htmlFor="machine-name" className="text-sm font-medium">
                    Name
                </label>
                <div className="flex gap-2">
                    <input
                        id="machine-name"
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        maxLength={100}
                        required
                        className="min-h-11 min-w-0 flex-1 rounded-[9px] border border-line-strong bg-bg px-3 text-ink"
                    />
                    <Button type="submit" disabled={busy || name.trim() === machine.name || !name.trim()}>
                        Save
                    </Button>
                </div>
            </form>
            {error && (
                <p role="alert" className="m-0 text-sm text-bad">
                    {error}
                </p>
            )}
            <div className="flex flex-col gap-3 border-t border-line pt-4">
                {confirming ? (
                    <div role="alert" className="flex flex-col gap-3 rounded-[10px] border border-bad-line bg-bad-bg p-3">
                        <p className="m-0 text-sm text-ink">
                            Remove {machine.name}? Its token stops working right away and its sessions disappear
                            from this server. You can pair it again later.
                        </p>
                        <div className="flex flex-wrap gap-2">
                            <Button variant="danger" onClick={() => void remove()} disabled={busy}>
                                Remove machine
                            </Button>
                            <Button variant="outline" onClick={() => setConfirming(false)}>
                                Keep it
                            </Button>
                        </div>
                    </div>
                ) : (
                    <div className="flex flex-wrap items-center justify-between gap-2">
                        <span className="text-sm text-dim">Unpair this machine from the server.</span>
                        <Button variant="danger" onClick={() => setConfirming(true)}>
                            Remove…
                        </Button>
                    </div>
                )}
            </div>
        </>
    );
}
