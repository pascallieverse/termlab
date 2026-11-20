import { useCallback, useEffect, useState, type FormEvent } from 'react';
import { Button } from '@/components/el/button';
import { Panel } from '@/components/el/panel';
import { Pill, type Tone } from '@/components/el/status';
import { useToast } from '@/components/el/toast';
import { AppShell, PageHeader } from '@/layouts/AppShell';
import { api, errorMessage } from '@/lib/api';
import { timeAgo } from '@/lib/format';
import { cn } from '@/lib/utils';
import type { WebhookDelivery, WebhookEndpoint } from '@/types';

type EventOption = { name: string; description: string };

const DEFAULT_EVENTS = ['session.needs_you', 'approval.requested', 'session.error'];

const HEALTH: Record<WebhookEndpoint['health'], { label: string; tone: Tone }> = {
    healthy: { label: 'Healthy', tone: 'ok' },
    failing: { label: 'Failing', tone: 'bad' },
    disabled: { label: 'Disabled', tone: 'off' },
};

const inputClass = 'min-h-11 rounded-[9px] border border-line-strong bg-bg px-3 text-ink';

export default function Webhooks() {
    const toast = useToast();
    const [endpoints, setEndpoints] = useState<WebhookEndpoint[] | null>(null);
    const [events, setEvents] = useState<EventOption[]>([]);
    const [selectedId, setSelectedId] = useState<number | null>(null);
    const [deliveries, setDeliveries] = useState<WebhookDelivery[]>([]);
    const [showForm, setShowForm] = useState(false);

    const load = useCallback(async () => {
        try {
            const data = await api.get<{ endpoints: WebhookEndpoint[]; events: EventOption[] }>('/api/webhooks');
            setEndpoints(data.endpoints);
            setEvents(data.events);
            setSelectedId((id) => (data.endpoints.some((e) => e.id === id) ? id : (data.endpoints[0]?.id ?? null)));
            if (data.endpoints.length === 0) {
                setShowForm(true);
            }
        } catch (e) {
            toast.error(errorMessage(e, 'Could not load webhooks.'));
        }
    }, [toast]);

    const loadDeliveries = useCallback(async (id: number) => {
        try {
            setDeliveries(await api.get<WebhookDelivery[]>(`/api/webhooks/${id}/deliveries`));
        } catch {
            setDeliveries([]);
        }
    }, []);

    useEffect(() => {
        void load();
    }, [load]);

    useEffect(() => {
        if (selectedId !== null) {
            void loadDeliveries(selectedId);
        } else {
            setDeliveries([]);
        }
    }, [selectedId, loadDeliveries]);

    const selected = endpoints?.find((e) => e.id === selectedId) ?? null;

    async function refresh() {
        await load();
        if (selectedId !== null) {
            await loadDeliveries(selectedId);
        }
    }

    return (
        <AppShell>
            <PageHeader
                title="Webhooks"
                description="Signed POST requests when your Termlab sessions change."
                actions={
                    <Button aria-expanded={showForm} aria-controls="new-endpoint" onClick={() => setShowForm(!showForm)}>
                        {showForm ? 'Close form' : 'Add endpoint'}
                    </Button>
                }
            />

            {showForm && (
                <NewEndpointForm
                    events={events}
                    canCancel={(endpoints?.length ?? 0) > 0}
                    onCancel={() => setShowForm(false)}
                    onSaved={async (endpoint) => {
                        setShowForm(false);
                        await load();
                        setSelectedId(endpoint.id);
                        toast.success('Endpoint added. Its signing secret is ready.');
                    }}
                />
            )}

            <Panel title="Endpoints" titleId="endpoints-heading">
                {endpoints === null ? (
                    <p className="m-0 px-5 py-6 text-dim">Loading…</p>
                ) : endpoints.length === 0 ? (
                    <p className="m-0 px-5 py-6 text-dim">
                        No endpoints yet. Add one to send session events to Slack, a CI job or your own server.
                    </p>
                ) : (
                    endpoints.map((ep) => {
                        const on = ep.id === selectedId;
                        const health = HEALTH[ep.health];

                        return (
                            <div
                                key={ep.id}
                                className={cn(
                                    'flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-t border-line-soft px-5 py-4 first:border-t-0',
                                    on && 'bg-selected shadow-[inset_3px_0_0_var(--color-amber)]',
                                )}
                            >
                                <button
                                    type="button"
                                    aria-pressed={on}
                                    onClick={() => setSelectedId(ep.id)}
                                    className="flex min-h-11 min-w-0 flex-[1_1_360px] cursor-pointer flex-col gap-1 border-0 bg-transparent p-0 text-left text-inherit"
                                >
                                    <span className="font-mono text-sm [overflow-wrap:anywhere]">{ep.url}</span>
                                    <span className="text-[13px] text-dim">
                                        {ep.description ? `${ep.description} · ` : ''}
                                        {events.length > 0 && ep.events.length === events.length
                                            ? 'All events'
                                            : ep.events.join(', ')}
                                    </span>
                                </button>
                                <div className="flex flex-wrap items-center gap-4">
                                    <span className="text-[13px] text-dim">
                                        {ep.last_delivery_at ? timeAgo(ep.last_delivery_at) : 'No deliveries yet'}
                                    </span>
                                    <Pill tone={health.tone}>{health.label}</Pill>
                                </div>
                            </div>
                        );
                    })
                )}
            </Panel>

            {selected && (
                <div className="grid grid-cols-[repeat(auto-fit,minmax(min(320px,100%),1fr))] items-start gap-5">
                    <SecretPanel key={selected.id} endpoint={selected} onChanged={refresh} />
                    <Deliveries
                        deliveries={deliveries}
                        onRefresh={() => void refresh()}
                        onRedeliver={async (id) => {
                            try {
                                await api.post(`/api/deliveries/${id}/redeliver`);
                                toast.success('Delivery queued again.');
                                window.setTimeout(() => void refresh(), 1500);
                            } catch (e) {
                                toast.error(errorMessage(e, 'Could not redeliver.'));
                            }
                        }}
                    />
                </div>
            )}
        </AppShell>
    );
}

function NewEndpointForm({
    events,
    canCancel,
    onCancel,
    onSaved,
}: {
    events: EventOption[];
    canCancel: boolean;
    onCancel: () => void;
    onSaved: (endpoint: WebhookEndpoint) => void | Promise<void>;
}) {
    const [url, setUrl] = useState('');
    const [description, setDescription] = useState('');
    const [chosen, setChosen] = useState<string[]>(DEFAULT_EVENTS);
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    function toggle(name: string) {
        setChosen((list) => (list.includes(name) ? list.filter((e) => e !== name) : [...list, name]));
    }

    async function submit(e: FormEvent) {
        e.preventDefault();
        if (chosen.length === 0) {
            setError('Pick at least one event.');

            return;
        }
        setBusy(true);
        setError(null);
        try {
            const endpoint = await api.post<WebhookEndpoint>('/api/webhooks', {
                url: url.trim(),
                description: description.trim() || null,
                events: chosen,
            });
            await onSaved(endpoint);
        } catch (err) {
            setError(errorMessage(err, 'Could not save the endpoint.'));
        } finally {
            setBusy(false);
        }
    }

    return (
        <section
            id="new-endpoint"
            aria-labelledby="new-heading"
            className="flex flex-col gap-5 rounded-[14px] border border-amber-line bg-panel p-6 max-sm:p-4"
        >
            <h2 id="new-heading" className="m-0 text-lg font-semibold">
                New endpoint
            </h2>
            <form onSubmit={submit} className="flex flex-col gap-5">
                <div className="grid grid-cols-[repeat(auto-fit,minmax(min(260px,100%),1fr))] gap-4">
                    <div className="flex flex-col gap-1.5">
                        <label htmlFor="url" className="text-sm font-medium">
                            Endpoint URL
                        </label>
                        <input
                            id="url"
                            type="url"
                            required
                            value={url}
                            onChange={(e) => setUrl(e.target.value)}
                            placeholder="https://hooks.example.com/termlab"
                            className={cn(inputClass, 'font-mono text-sm')}
                        />
                    </div>
                    <div className="flex flex-col gap-1.5">
                        <label htmlFor="desc" className="text-sm font-medium">
                            Description <span className="font-normal text-dim">(optional)</span>
                        </label>
                        <input
                            id="desc"
                            type="text"
                            maxLength={200}
                            value={description}
                            onChange={(e) => setDescription(e.target.value)}
                            placeholder="Ping #agents in Slack"
                            className={inputClass}
                        />
                    </div>
                </div>

                <fieldset className="m-0 flex flex-col gap-2.5 border-0 p-0">
                    <legend className="pb-2.5 text-sm font-medium">Events</legend>
                    <div className="grid grid-cols-[repeat(auto-fit,minmax(min(260px,100%),1fr))] gap-2">
                        {events.map((ev) => (
                            <label
                                key={ev.name}
                                className="flex cursor-pointer items-start gap-3 rounded-[10px] border border-line bg-surface p-3"
                            >
                                <input
                                    type="checkbox"
                                    checked={chosen.includes(ev.name)}
                                    onChange={() => toggle(ev.name)}
                                    className="mt-0.5 size-[18px] accent-amber"
                                />
                                <span className="flex flex-col gap-0.5">
                                    <span className="font-mono text-[13px]">{ev.name}</span>
                                    <span className="text-[13px] text-dim">{ev.description}</span>
                                </span>
                            </label>
                        ))}
                    </div>
                </fieldset>

                {error && (
                    <p role="alert" className="m-0 text-sm text-bad">
                        {error}
                    </p>
                )}

                <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-5">
                    <span className="text-sm text-dim">A signing secret is created when you save.</span>
                    <div className="flex gap-2">
                        {canCancel && (
                            <Button variant="outline" onClick={onCancel}>
                                Cancel
                            </Button>
                        )}
                        <Button type="submit" disabled={busy}>
                            Save endpoint
                        </Button>
                    </div>
                </div>
            </form>
        </section>
    );
}

function SecretPanel({ endpoint, onChanged }: { endpoint: WebhookEndpoint; onChanged: () => Promise<void> }) {
    const toast = useToast();
    const [secret, setSecret] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const [confirm, setConfirm] = useState<'roll' | 'delete' | null>(null);

    async function reveal(path: 'secret' | 'roll') {
        setBusy(true);
        try {
            const data = await api.post<{ secret: string }>(`/api/webhooks/${endpoint.id}/${path}`);
            setSecret(data.secret);
            if (path === 'roll') {
                toast.success('New secret created. Update your receiver.');
            }
        } catch (e) {
            toast.error(errorMessage(e, 'Could not load the secret.'));
        } finally {
            setBusy(false);
            setConfirm(null);
        }
    }

    async function setEnabled(enabled: boolean) {
        try {
            await api.patch(`/api/webhooks/${endpoint.id}`, { enabled });
            await onChanged();
        } catch (e) {
            toast.error(errorMessage(e, 'Could not change the endpoint.'));
        }
    }

    async function remove() {
        setBusy(true);
        try {
            await api.del(`/api/webhooks/${endpoint.id}`);
            toast.success('Endpoint deleted.');
            await onChanged();
        } catch (e) {
            toast.error(errorMessage(e, 'Could not delete the endpoint.'));
            setBusy(false);
        }
    }

    async function test() {
        try {
            await api.post(`/api/webhooks/${endpoint.id}/test`);
            toast.success('Test event queued.');
            window.setTimeout(() => void onChanged(), 1500);
        } catch (e) {
            toast.error(errorMessage(e, 'Could not send a test event.'));
        }
    }

    async function copy() {
        if (!secret) {
            return;
        }
        try {
            await navigator.clipboard.writeText(secret);
            toast.success('Secret copied.');
        } catch {
            toast.error('Could not copy. Select the secret and copy it.');
        }
    }

    return (
        <section aria-labelledby="secret-heading" className="flex flex-col gap-3.5 rounded-[14px] border border-line bg-panel p-5">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <h2 id="secret-heading" className="m-0 text-[17px] font-semibold">
                    Signing secret
                </h2>
                <label className="flex cursor-pointer items-center gap-2 text-sm text-dim">
                    <input
                        type="checkbox"
                        checked={endpoint.enabled}
                        onChange={(e) => void setEnabled(e.target.checked)}
                        className="size-[18px] accent-amber"
                    />
                    Enabled
                </label>
            </div>
            <p className="m-0 text-sm text-dim">
                For <span className="font-mono [overflow-wrap:anywhere] text-ink-soft">{endpoint.url}</span>. Check the{' '}
                <span className="font-mono text-ink-soft">X-Termlab-Signature</span> header against it.
            </p>
            <div className="flex flex-wrap gap-2">
                <code className="min-w-0 flex-[1_1_220px] overflow-x-auto rounded-lg border border-line bg-bg p-3 font-mono text-[13px] text-ink-soft">
                    {secret ?? 'whsec_••••••••••••••••••••'}
                </code>
                <Button variant="outline" disabled={busy} onClick={() => (secret ? setSecret(null) : void reveal('secret'))}>
                    {secret ? 'Hide' : 'Reveal'}
                </Button>
                {secret && (
                    <Button variant="outline" onClick={() => void copy()}>
                        Copy
                    </Button>
                )}
            </div>

            {confirm ? (
                <div role="alert" className="flex flex-col gap-3 rounded-[10px] border border-bad-line bg-bad-bg p-3">
                    <p className="m-0 text-sm text-ink">
                        {confirm === 'roll'
                            ? 'Roll the secret? The old one stops working right away, so update your receiver.'
                            : 'Delete this endpoint and its delivery log? This cannot be undone.'}
                    </p>
                    <div className="flex flex-wrap gap-2">
                        <Button
                            variant="danger"
                            disabled={busy}
                            onClick={() => (confirm === 'roll' ? void reveal('roll') : void remove())}
                        >
                            {confirm === 'roll' ? 'Roll secret' : 'Delete endpoint'}
                        </Button>
                        <Button variant="outline" onClick={() => setConfirm(null)}>
                            Cancel
                        </Button>
                    </div>
                </div>
            ) : (
                <div className="flex flex-wrap gap-2">
                    <Button variant="outline" disabled={!endpoint.enabled} onClick={() => void test()}>
                        Send test event
                    </Button>
                    <Button variant="outline" onClick={() => setConfirm('roll')}>
                        Roll secret
                    </Button>
                    <Button variant="danger" onClick={() => setConfirm('delete')}>
                        Delete endpoint
                    </Button>
                </div>
            )}
        </section>
    );
}

function Deliveries({
    deliveries,
    onRefresh,
    onRedeliver,
}: {
    deliveries: WebhookDelivery[];
    onRefresh: () => void;
    onRedeliver: (id: number) => void;
}) {
    return (
        <section aria-labelledby="log-heading" className="overflow-hidden rounded-[14px] border border-line bg-panel">
            <div className="flex items-center justify-between gap-3 border-b border-line px-5 py-[18px]">
                <h2 id="log-heading" className="m-0 text-[17px] font-semibold">
                    Recent deliveries
                </h2>
                <button
                    type="button"
                    onClick={onRefresh}
                    className="cursor-pointer rounded-md px-2 py-1 text-[13px] text-dim hover:bg-raised hover:text-ink"
                >
                    Refresh
                </button>
            </div>
            {deliveries.length === 0 ? (
                <p className="m-0 px-5 py-6 text-dim">No deliveries yet. Send a test event to try the endpoint.</p>
            ) : (
                <div className="relative overflow-x-auto">
                    <table className="w-full min-w-[520px] border-collapse text-sm">
                        <thead>
                            <tr className="text-left text-[13px] text-dim">
                                <th scope="col" className="px-5 py-2.5 font-medium">Event</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">Response</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">When</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">
                                    <span className="sr-only">Actions</span>
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {deliveries.map((d) => (
                                <tr key={d.id} className="border-t border-line-soft">
                                    <td className="px-5 py-3 font-mono text-[13px]">{d.event}</td>
                                    <td className="px-5 py-3">
                                        <Pill tone={d.pending ? 'off' : d.ok ? 'ok' : 'bad'}>
                                            {d.pending ? 'pending' : (d.response_code ?? '—')}
                                        </Pill>
                                        {d.attempt > 1 && <span className="ml-2 text-xs text-dim">attempt {d.attempt}</span>}
                                    </td>
                                    <td className="px-5 py-3 text-dim">{timeAgo(d.delivered_at ?? d.created_at)}</td>
                                    <td className="px-5 py-3 text-right">
                                        <Button variant="outline" size="sm" disabled={d.pending} onClick={() => onRedeliver(d.id)}>
                                            Redeliver
                                        </Button>
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </section>
    );
}
