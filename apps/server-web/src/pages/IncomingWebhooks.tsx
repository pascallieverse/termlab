import { useCallback, useEffect, useState, type FormEvent } from 'react';
import { Link } from 'react-router-dom';
import { Button } from '@/components/el/button';
import { Panel } from '@/components/el/panel';
import { Segmented } from '@/components/el/segmented';
import { Pill, type Tone } from '@/components/el/status';
import { useToast } from '@/components/el/toast';
import { HeadActions } from '@/layouts/SettingsLayout';
import { api, errorMessage } from '@/lib/api';
import { timeAgo } from '@/lib/format';
import { cn } from '@/lib/utils';
import { useLive } from '@/live/LiveProvider';
import type { IncomingCall, IncomingHook, PermissionPreset, SessionLabel } from '@/types';
import { Field, LabelChip, SecretValue, inputClass, labelHex, selectClass, useScrollIntoView } from './webhook-ui';

const PERMISSIONS: { value: PermissionPreset; label: string; desc: string }[] = [
    { value: 'ask_first', label: 'Ask first', desc: 'You approve every edit and command, from here or the app.' },
    { value: 'auto_edit', label: 'Auto-edit', desc: 'Edits go ahead; commands wait for you.' },
    {
        value: 'full_auto',
        label: 'Full auto',
        desc: 'No prompts. Anyone with the URL can then run commands on this computer through the agent.',
    },
];

const PERMISSION_LABEL = Object.fromEntries(PERMISSIONS.map((p) => [p.value, p.label])) as Record<PermissionPreset, string>;

const CALL: Record<IncomingCall['status'], { label: string; tone: Tone }> = {
    started: { label: 'Started', tone: 'ok' },
    pending: { label: 'Starting', tone: 'off' },
    failed: { label: 'Failed', tone: 'bad' },
    offline: { label: 'Offline', tone: 'bad' },
    disabled: { label: 'Turned off', tone: 'off' },
    no_answer: { label: 'No answer', tone: 'bad' },
};

/** Settings › Webhooks › Incoming: URLs that start sessions. */
export function IncomingWebhooks() {
    const toast = useToast();
    const live = useLive();
    const [hooks, setHooks] = useState<IncomingHook[] | null>(null);
    const [selectedId, setSelectedId] = useState<number | null>(null);
    const [calls, setCalls] = useState<IncomingCall[]>([]);
    const [form, setForm] = useState<'new' | IncomingHook | null>(null);
    // Each computer's labels, by machine id.
    const [labels, setLabels] = useState<Record<string, SessionLabel[]>>({});

    const load = useCallback(async () => {
        try {
            const data = await api.get<{ hooks: IncomingHook[]; labels?: Record<string, SessionLabel[]> }>('/api/incoming');
            setHooks(data.hooks);
            setLabels(data.labels ?? {});
            // The open one stays open; a single webhook opens by itself.
            setSelectedId((id) =>
                data.hooks.some((h) => h.id === id) ? id : data.hooks.length === 1 ? data.hooks[0].id : null,
            );
            if (data.hooks.length === 0) setForm((f) => f ?? 'new');
        } catch (e) {
            toast.error(errorMessage(e, 'Could not load incoming webhooks.'));
        }
    }, [toast]);

    const loadCalls = useCallback(async (id: number) => {
        try {
            setCalls(await api.get<IncomingCall[]>(`/api/incoming/${id}/calls`));
        } catch {
            setCalls([]);
        }
    }, []);

    useEffect(() => {
        void load();
    }, [load]);

    useEffect(() => {
        if (selectedId !== null) void loadCalls(selectedId);
        else setCalls([]);
    }, [selectedId, loadCalls]);

    async function refresh() {
        await load();
        if (selectedId !== null) await loadCalls(selectedId);
    }

    const [deleting, setDeleting] = useState<number | null>(null);

    async function setEnabled(h: IncomingHook, enabled: boolean) {
        try {
            await api.patch(`/api/incoming/${h.id}`, { enabled });
            await load();
        } catch (e) {
            toast.error(errorMessage(e, 'Could not change the webhook.'));
        }
    }

    async function remove(h: IncomingHook) {
        try {
            await api.del(`/api/incoming/${h.id}`);
            setDeleting(null);
            toast.success('Incoming webhook deleted.');
            await load();
        } catch (e) {
            toast.error(errorMessage(e, 'Could not delete the webhook.'));
        }
    }

    const agentName = (h: IncomingHook) =>
        live.projects[h.machine_id]?.agents.find((a) => a.agent === h.agent)?.name ?? h.agent;

    return (
        <>
            <HeadActions>
                <Button aria-expanded={form !== null} aria-controls="hook-form" onClick={() => setForm(form ? null : 'new')}>
                    {form ? 'Close form' : 'Add incoming webhook'}
                </Button>
            </HeadActions>

            {form && (
                <HookForm
                    key={form === 'new' ? 'new' : form.id}
                    initial={form === 'new' ? null : form}
                    labels={labels}
                    onLabels={(id, list) => setLabels((all) => ({ ...all, [id]: list }))}
                    canCancel={(hooks?.length ?? 0) > 0}
                    onCancel={() => setForm(null)}
                    onSaved={async (hook, created) => {
                        setForm(null);
                        await load();
                        setSelectedId(hook.id);
                        toast.success(created ? 'Incoming webhook added. Reveal its URL to use it.' : 'Changes saved.');
                    }}
                />
            )}

            <Panel title="Incoming webhooks" titleId="incoming-heading">
                {hooks === null ? (
                    <p className="m-0 px-5 py-6 text-dim">Loading…</p>
                ) : hooks.length === 0 ? (
                    <p className="m-0 px-5 py-6 text-dim">
                        None yet. An incoming webhook gives you a URL: when GitHub, a CI job, a form or a phone shortcut
                        calls it, Termlab starts a session with the request as its first message.
                    </p>
                ) : (
                    hooks.map((h) => {
                        const on = h.id === selectedId;

                        return (
                            <div
                                key={h.id}
                                className={cn(
                                    'border-t border-line-soft first:border-t-0',
                                    on && 'bg-selected shadow-[inset_3px_0_0_var(--color-amber)]',
                                )}
                            >
                                <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3 px-5 py-4">
                                    <button
                                        type="button"
                                        aria-expanded={on}
                                        aria-controls={`hook-${h.id}`}
                                        onClick={() => setSelectedId(on ? null : h.id)}
                                        className="flex min-h-11 min-w-0 flex-[1_1_360px] cursor-pointer flex-col gap-1 border-0 bg-transparent p-0 text-left text-inherit"
                                    >
                                        <span className="flex items-center gap-2 text-sm font-medium">
                                            <span aria-hidden="true" className={cn('text-xs text-dim transition-transform', on && 'rotate-90')}>
                                                ▶
                                            </span>
                                            {h.name}
                                        </span>
                                        <span className="truncate font-mono text-xs text-faint">{h.url_hint}</span>
                                        <span className="text-[13px] text-dim">
                                            {agentName(h)} in {h.project_template ? 'the project the request names' : h.project}
                                            {h.machine ? ` on ${h.machine}` : ''} · {PERMISSION_LABEL[h.permission]}
                                        </span>
                                        {(h.label_ids.length > 0 || h.labels_template) && (
                                            <span className="flex flex-wrap items-center gap-1.5">
                                                {h.label_ids.map((id) => {
                                                    const l = labels[h.machine_id]?.find((x) => x.id === id);

                                                    return l ? <LabelChip key={id} name={l.name} color={l.color} /> : null;
                                                })}
                                                {h.labels_template && <span className="text-xs text-dim">+ labels from the request</span>}
                                            </span>
                                        )}
                                    </button>
                                    <div className="flex flex-wrap items-center gap-4">
                                        <span className="text-[13px] text-dim">
                                            {h.last_call_at ? `Called ${timeAgo(h.last_call_at)}` : 'Not called yet'}
                                        </span>
                                        {h.project_missing && <Pill tone="bad">Project missing</Pill>}
                                        <button
                                            type="button"
                                            role="switch"
                                            aria-checked={h.enabled}
                                            aria-label={`${h.name}: ${h.enabled ? 'on' : 'off'}`}
                                            title={h.enabled ? 'Turn off: calls get 403' : 'Turn on'}
                                            onClick={() => void setEnabled(h, !h.enabled)}
                                            className="cursor-pointer border-0 bg-transparent p-0"
                                        >
                                            <Pill tone={h.enabled ? 'ok' : 'off'}>{h.enabled ? 'On' : 'Off'}</Pill>
                                        </button>
                                        <Button variant="outline" size="sm" onClick={() => setForm(h)}>
                                            Edit
                                        </Button>
                                        <Button variant="danger" size="sm" onClick={() => setDeleting(h.id)}>
                                            Delete
                                        </Button>
                                    </div>
                                </div>
                                {deleting === h.id && (
                                    <div role="alert" className="mx-5 mb-4 flex flex-wrap items-center justify-between gap-3 rounded-[10px] border border-bad-line bg-bad-bg p-3">
                                        <p className="m-0 text-sm text-ink">
                                            Delete “{h.name}” and its call log? Its URL stops working.
                                        </p>
                                        <div className="flex gap-2">
                                            <Button variant="danger" size="sm" onClick={() => void remove(h)}>
                                                Delete webhook
                                            </Button>
                                            <Button variant="outline" size="sm" onClick={() => setDeleting(null)}>
                                                Cancel
                                            </Button>
                                        </div>
                                    </div>
                                )}
                                {on && (
                                    <div
                                        id={`hook-${h.id}`}
                                        className="grid grid-cols-[repeat(auto-fit,minmax(min(320px,100%),1fr))] items-start gap-4 px-5 pb-5"
                                    >
                                        <UrlPanel key={h.id} hook={h} onChanged={refresh} />
                                        <Calls calls={calls} onRefresh={() => void refresh()} />
                                    </div>
                                )}
                            </div>
                        );
                    })
                )}
            </Panel>
        </>
    );
}

type ProjectMode = 'fixed' | 'request';

const CUSTOM_MODEL = '__custom__';

function HookForm({
    initial,
    labels: allLabels,
    onLabels,
    canCancel,
    onCancel,
    onSaved,
}: {
    initial: IncomingHook | null;
    labels: Record<string, SessionLabel[]>;
    /** A label was made on a computer: its labels now. */
    onLabels: (machineId: number, labels: SessionLabel[]) => void;
    canCancel: boolean;
    onCancel: () => void;
    onSaved: (hook: IncomingHook, created: boolean) => void | Promise<void>;
}) {
    const live = useLive();
    // Computers whose Termlab reported its projects (online or not: the hook waits for it).
    const machines = live.machines.filter((m) => (live.projects[m.id]?.projects.length ?? 0) > 0);
    const [name, setName] = useState(initial?.name ?? '');
    const [machineId, setMachineId] = useState<number | null>(initial?.machine_id ?? machines[0]?.id ?? null);
    const mp = machineId !== null ? live.projects[machineId] : undefined;
    const [projectMode, setProjectMode] = useState<ProjectMode>(initial?.project_template ? 'request' : 'fixed');
    const [projectTemplate, setProjectTemplate] = useState(initial?.project_template || '{{body.project}}');
    // The fixed project, or the fallback for a request's that doesn't match (0: none).
    const [folderId, setFolderId] = useState<number>(
        initial ? (initial.folder_id ?? 0) : (mp?.projects[0]?.id ?? 0),
    );
    const [labelIds, setLabelIds] = useState<number[]>(initial?.label_ids ?? []);
    const [labelsTemplate, setLabelsTemplate] = useState(initial?.labels_template ?? '');
    const [createLabels, setCreateLabels] = useState(initial?.create_labels ?? false);
    const [agent, setAgent] = useState<string | null>(
        initial?.agent ?? mp?.agents.find((a) => a.agent !== 'shell')?.agent ?? mp?.agents[0]?.agent ?? null,
    );
    const [permission, setPermission] = useState<PermissionPreset>(initial?.permission ?? 'ask_first');
    const [model, setModel] = useState(initial?.model ?? '');
    // A model the agent doesn't list: typed in.
    const [customModel, setCustomModel] = useState(
        !!initial?.model && !(mp?.agents.find((a) => a.agent === initial.agent)?.models ?? []).includes(initial.model),
    );
    const [title, setTitle] = useState(initial?.title ?? '');
    const [message, setMessage] = useState(initial?.message ?? '{{body}}');
    const [notify, setNotify] = useState(initial?.notify ?? true);
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    const info = mp?.agents.find((a) => a.agent === agent);
    const shell = agent === 'shell';
    const box = useScrollIntoView<HTMLElement>();

    const machineLabels = machineId !== null ? (allLabels[machineId] ?? []) : [];
    const machineName = live.machines.find((m) => m.id === machineId)?.name ?? 'this computer';

    function pickMachine(id: number) {
        const next = live.projects[id];
        setMachineId(id);
        setFolderId(projectMode === 'request' ? 0 : (next?.projects[0]?.id ?? 0));
        // Labels are each computer's own.
        setLabelIds([]);
        setAgent(next?.agents.find((a) => a.agent === agent)?.agent ?? next?.agents[0]?.agent ?? null);
    }

    async function submit(e: FormEvent) {
        e.preventDefault();
        const request = projectMode === 'request';
        if (machineId === null || !agent || (!request && !folderId) || (request && !projectTemplate.trim())) {
            setError(request ? 'Say where the project comes from, like {{body.project}}.' : 'Pick a computer, a project and an agent.');

            return;
        }
        setBusy(true);
        setError(null);
        const body = {
            name: name.trim(),
            machine_id: machineId,
            folder_id: folderId,
            project_template: request ? projectTemplate.trim() : '',
            label_ids: labelIds.filter((id) => machineLabels.some((l) => l.id === id)),
            labels_template: labelsTemplate.trim(),
            create_labels: createLabels,
            agent,
            permission,
            model: info?.model_flag ? model.trim() : '',
            title: title.trim(),
            message,
            notify: notify && !shell,
        };
        try {
            const hook = initial
                ? await api.patch<IncomingHook>(`/api/incoming/${initial.id}`, body)
                : await api.post<IncomingHook>('/api/incoming', body);
            await onSaved(hook, !initial);
        } catch (err) {
            setError(errorMessage(err, 'Could not save the webhook.'));
        } finally {
            setBusy(false);
        }
    }

    if (machines.length === 0) {
        return (
            <section ref={box} id="hook-form" className="rounded-[14px] border border-amber-line bg-panel p-6 text-sm text-dim max-sm:p-4">
                Pair a computer running Termlab 0.2.9 or newer first: an incoming webhook starts its sessions there.
            </section>
        );
    }

    return (
        <section
            ref={box}
            id="hook-form"
            aria-labelledby="hook-form-heading"
            className="flex flex-col gap-5 rounded-[14px] border border-amber-line bg-panel p-6 max-sm:p-4"
        >
            <h2 id="hook-form-heading" className="m-0 text-lg font-semibold">
                {initial ? `Edit ${initial.name}` : 'New incoming webhook'}
            </h2>
            <form onSubmit={submit} className="flex flex-col gap-5">
                <div className="grid grid-cols-[repeat(auto-fit,minmax(min(260px,100%),1fr))] gap-4">
                    <Field id="hook-name" label="Name">
                        <input
                            id="hook-name"
                            required
                            maxLength={100}
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                            placeholder="Fix GitHub issues"
                            className={inputClass}
                        />
                    </Field>
                    <Field id="hook-machine" label="Computer">
                        <select
                            id="hook-machine"
                            value={machineId ?? ''}
                            onChange={(e) => pickMachine(Number(e.target.value))}
                            className={selectClass}
                        >
                            {machines.map((m) => (
                                <option key={m.id} value={m.id}>
                                    {m.name}
                                    {m.online ? '' : ' (offline)'}
                                </option>
                            ))}
                        </select>
                    </Field>
                    <Field id="hook-agent" label="Agent">
                        <select
                            id="hook-agent"
                            value={agent ?? ''}
                            onChange={(e) => {
                                setAgent(e.target.value);
                                setModel('');
                                setCustomModel(false);
                            }}
                            className={selectClass}
                        >
                            {mp?.agents.map((a) => (
                                <option key={a.agent} value={a.agent}>
                                    {a.name}
                                </option>
                            ))}
                        </select>
                    </Field>
                    {!shell && (
                        <Field id="hook-model" label="Model">
                            <select
                                id="hook-model"
                                value={info?.model_flag ? (customModel ? CUSTOM_MODEL : model) : ''}
                                disabled={!info?.model_flag}
                                onChange={(e) => {
                                    const v = e.target.value;
                                    setCustomModel(v === CUSTOM_MODEL);
                                    if (v !== CUSTOM_MODEL) setModel(v);
                                }}
                                className={selectClass}
                            >
                                {info?.model_flag ? (
                                    <>
                                        <option value="">Default (the agent's own setting)</option>
                                        {info.models.map((m) => (
                                            <option key={m} value={m}>
                                                {m}
                                            </option>
                                        ))}
                                        <option value={CUSTOM_MODEL}>Other…</option>
                                    </>
                                ) : (
                                    <option value="">Set in {info?.name ?? 'the agent'}'s own config</option>
                                )}
                            </select>
                            {customModel && info?.model_flag && (
                                <input
                                    aria-label="Model name"
                                    value={model}
                                    onChange={(e) => setModel(e.target.value)}
                                    placeholder="claude-opus-5-5"
                                    autoFocus
                                    className={cn(inputClass, 'font-mono text-sm')}
                                />
                            )}
                        </Field>
                    )}
                </div>

                <fieldset className="m-0 flex flex-col gap-2.5 border-0 p-0">
                    <legend className="pb-2 text-sm font-medium">Project</legend>
                    <Segmented<ProjectMode>
                        label="Where the project comes from"
                        options={[
                            { value: 'fixed', label: 'One project' },
                            { value: 'request', label: 'From the request' },
                        ]}
                        value={projectMode}
                        onChange={(m) => {
                            setProjectMode(m);
                            if (m === 'fixed' && !folderId) setFolderId(mp?.projects[0]?.id ?? 0);
                        }}
                        className="self-start"
                    />
                    {projectMode === 'fixed' ? (
                        <select
                            id="hook-project"
                            aria-label="Project"
                            value={folderId}
                            onChange={(e) => setFolderId(Number(e.target.value))}
                            className={cn(selectClass, 'max-w-[420px]')}
                        >
                            <ProjectOptions initial={initial} folderId={folderId} projects={mp?.projects ?? []} />
                        </select>
                    ) : (
                        <div className="grid grid-cols-[repeat(auto-fit,minmax(min(260px,100%),1fr))] gap-4">
                            <Field
                                id="hook-project-template"
                                label="Project name"
                                hint={`Matched to a project's name or folder name (any capitals) or its full path on ${machineName}.`}
                            >
                                <input
                                    id="hook-project-template"
                                    value={projectTemplate}
                                    onChange={(e) => setProjectTemplate(e.target.value)}
                                    placeholder="{{body.repository.name}}"
                                    className={cn(inputClass, 'font-mono text-sm')}
                                />
                            </Field>
                            <Field id="hook-project-fallback" label="When nothing matches">
                                <select
                                    id="hook-project-fallback"
                                    value={folderId}
                                    onChange={(e) => setFolderId(Number(e.target.value))}
                                    className={selectClass}
                                >
                                    <option value={0}>Refuse the call (422)</option>
                                    <ProjectOptions initial={initial} folderId={folderId} projects={mp?.projects ?? []} prefix="Use " />
                                </select>
                            </Field>
                        </div>
                    )}
                </fieldset>

                <fieldset className="m-0 flex flex-col gap-2.5 border-0 p-0">
                    <legend className="pb-2 text-sm font-medium">
                        Labels <span className="font-normal text-dim">(optional)</span>
                    </legend>
                    <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Labels every session gets">
                        {machineLabels.map((l) => {
                                const on = labelIds.includes(l.id);

                                return (
                                    <button
                                        key={l.id}
                                        type="button"
                                        aria-pressed={on}
                                        onClick={() => setLabelIds((ids) => (on ? ids.filter((x) => x !== l.id) : [...ids, l.id]))}
                                        className={cn(
                                            'inline-flex min-h-9 cursor-pointer items-center gap-2 rounded-full border px-3 text-[13px]',
                                            on ? 'bg-raised text-ink' : 'border-line bg-transparent text-dim hover:text-ink',
                                        )}
                                        style={on ? { borderColor: labelHex(l.color) } : undefined}
                                    >
                                        <span aria-hidden="true" className="size-2.5 rounded-full" style={{ background: labelHex(l.color) }} />
                                        {l.name}
                                        {on && <span aria-hidden="true">✓</span>}
                                    </button>
                                );
                            })}
                        {machineId !== null && (
                            <NewLabel
                                machineId={machineId}
                                onMade={(label, list) => {
                                    onLabels(machineId, list);
                                    setLabelIds((ids) => (ids.includes(label.id) ? ids : [...ids, label.id]));
                                }}
                            />
                        )}
                    </div>
                    {machineLabels.length === 0 && (
                        <span className="text-[13px] text-dim">
                            No labels on {machineName} yet. New label makes one there, the same as in Termlab.
                        </span>
                    )}
                    <Field
                        id="hook-labels-template"
                        label="Labels from the request"
                        optional
                        hint="Label names, as a JSON list or separated by commas, matched whatever their capitals."
                    >
                        <input
                            id="hook-labels-template"
                            value={labelsTemplate}
                            onChange={(e) => setLabelsTemplate(e.target.value)}
                            placeholder="{{body.labels}}"
                            className={cn(inputClass, 'max-w-[420px] font-mono text-sm')}
                        />
                    </Field>
                    {labelsTemplate.trim() && (
                        <label className="flex cursor-pointer items-center gap-2 text-sm">
                            <input
                                type="checkbox"
                                checked={createLabels}
                                onChange={(e) => setCreateLabels(e.target.checked)}
                                className="size-[18px] accent-amber"
                            />
                            Make labels that don't exist yet (grey)
                        </label>
                    )}
                </fieldset>

                <fieldset className="m-0 flex flex-col gap-2 border-0 p-0">
                    <legend className="pb-2 text-sm font-medium">Permissions</legend>
                    <Segmented label="Permissions" options={PERMISSIONS} value={permission} onChange={setPermission} className="self-start" />
                    <span className={cn('text-[13px]', permission === 'full_auto' ? 'text-bad' : 'text-dim')}>
                        {PERMISSIONS.find((p) => p.value === permission)?.desc}
                    </span>
                </fieldset>

                <Field
                    id="hook-message"
                    label={shell ? 'Command' : 'First message'}
                    hint={<TemplateHelp />}
                >
                    <textarea
                        id="hook-message"
                        rows={4}
                        value={message}
                        onChange={(e) => setMessage(e.target.value)}
                        placeholder={'Fix this issue: {{body.issue.title}}\n\n{{body.issue.body}}'}
                        className={cn(inputClass, 'py-2.5 font-mono text-sm')}
                    />
                </Field>
                <Field
                    id="hook-title"
                    label="Session title"
                    optional
                    hint="Blank: Termlab names it as usual. Needs Termlab 0.2.14 on the computer."
                >
                    <input
                        id="hook-title"
                        maxLength={500}
                        value={title}
                        onChange={(e) => setTitle(e.target.value)}
                        placeholder="#{{body.issue.number}} {{body.issue.title}}"
                        className={cn(inputClass, 'font-mono text-sm')}
                    />
                </Field>

                {!shell && (
                    <label className="flex cursor-pointer items-center gap-2 text-sm">
                        <input type="checkbox" checked={notify} onChange={(e) => setNotify(e.target.checked)} className="size-[18px] accent-amber" />
                        Notify me when the session needs input
                    </label>
                )}

                {error && (
                    <p role="alert" className="m-0 text-sm text-bad">
                        {error}
                    </p>
                )}

                <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-5">
                    <span className="text-sm text-dim">
                        {initial ? 'The URL stays the same.' : 'Its URL is created when you save.'}
                    </span>
                    <div className="flex gap-2">
                        {(canCancel || initial) && (
                            <Button variant="outline" onClick={onCancel}>
                                Cancel
                            </Button>
                        )}
                        <Button type="submit" disabled={busy}>
                            {initial ? 'Save changes' : 'Create webhook'}
                        </Button>
                    </div>
                </div>
            </form>
        </section>
    );
}

const LABEL_COLOR_NAMES = ['red', 'orange', 'yellow', 'green', 'cyan', 'blue', 'purple', 'pink', 'gray'];

/** Makes a label on the computer (Termlab keeps them), from a name and a colour. */
function NewLabel({ machineId, onMade }: { machineId: number; onMade: (label: SessionLabel, labels: SessionLabel[]) => void }) {
    const [open, setOpen] = useState(false);
    const [name, setName] = useState('');
    const [color, setColor] = useState('blue');
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    async function make() {
        if (!name.trim()) return;
        setBusy(true);
        setError(null);
        try {
            const data = await api.post<{ label: SessionLabel; labels: SessionLabel[] }>(`/api/machines/${machineId}/labels`, {
                name: name.trim(),
                color,
            });
            onMade(data.label, data.labels);
            setName('');
            setOpen(false);
        } catch (e) {
            setError(errorMessage(e, 'Could not make the label.'));
        } finally {
            setBusy(false);
        }
    }

    if (!open) {
        return (
            <button
                type="button"
                onClick={() => setOpen(true)}
                className="inline-flex min-h-9 cursor-pointer items-center gap-1.5 rounded-full border border-dashed border-line-strong bg-transparent px-3 text-[13px] text-dim hover:text-ink"
            >
                <span aria-hidden="true">+</span> New label
            </button>
        );
    }

    return (
        <div className="flex basis-full flex-col gap-2 rounded-[10px] border border-line bg-surface p-3">
            <div className="flex flex-wrap items-center gap-2">
                <input
                    aria-label="Label name"
                    value={name}
                    maxLength={32}
                    autoFocus
                    onChange={(e) => setName(e.target.value)}
                    onKeyDown={(e) => {
                        // Enter makes the label, not the webhook.
                        if (e.key === 'Enter') {
                            e.preventDefault();
                            void make();
                        } else if (e.key === 'Escape') {
                            e.preventDefault();
                            setOpen(false);
                        }
                    }}
                    placeholder="Label name"
                    className={cn(inputClass, 'min-h-9 w-[200px] text-sm')}
                />
                <div role="radiogroup" aria-label="Colour" className="flex flex-wrap gap-1">
                    {LABEL_COLOR_NAMES.map((c) => (
                        <button
                            key={c}
                            type="button"
                            role="radio"
                            aria-checked={color === c}
                            aria-label={c}
                            title={c}
                            onClick={() => setColor(c)}
                            className={cn(
                                'grid size-7 cursor-pointer place-items-center rounded-full border-2 bg-transparent',
                                color === c ? 'border-ink' : 'border-transparent',
                            )}
                        >
                            <span className="size-4 rounded-full" style={{ background: labelHex(c) }} />
                        </button>
                    ))}
                </div>
                <Button size="sm" disabled={busy || !name.trim()} onClick={() => void make()}>
                    {busy ? 'Making…' : 'Make label'}
                </Button>
                <Button size="sm" variant="outline" onClick={() => setOpen(false)}>
                    Cancel
                </Button>
            </div>
            {error && (
                <p role="alert" className="m-0 text-[13px] text-bad">
                    {error}
                </p>
            )}
        </div>
    );
}

/** The computer's projects, plus the saved one when its folder is gone. */
function ProjectOptions({
    initial,
    folderId,
    projects,
    prefix = '',
}: {
    initial: IncomingHook | null;
    folderId: number;
    projects: { id: number; name: string }[];
    prefix?: string;
}) {
    return (
        <>
            {initial?.project_missing && initial.folder_id !== null && folderId === initial.folder_id && (
                <option value={initial.folder_id}>
                    {prefix}
                    {initial.project} (missing)
                </option>
            )}
            {projects.map((p) => (
                <option key={p.id} value={p.id}>
                    {prefix}
                    {p.name}
                </option>
            ))}
        </>
    );
}

function TemplateHelp() {
    const code = 'rounded bg-raised px-1 font-mono text-[12px] text-ink-soft';

    return (
        <>
            Filled from the request: <code className={code}>{'{{body}}'}</code> the whole body,{' '}
            <code className={code}>{'{{body.issue.title}}'}</code> a JSON or form field (<code className={code}>.0</code> for a
            list item), <code className={code}>{'{{query.name}}'}</code> and <code className={code}>{'{{headers.name}}'}</code>.
            Blank: the session starts without a message.
        </>
    );
}

function UrlPanel({ hook, onChanged }: { hook: IncomingHook; onChanged: () => Promise<void> }) {
    const toast = useToast();
    const [url, setUrl] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const [confirm, setConfirm] = useState(false);

    async function reveal(path: 'url' | 'roll') {
        setBusy(true);
        try {
            const data = await api.post<{ url: string }>(`/api/incoming/${hook.id}/${path}`);
            setUrl(data.url);
            if (path === 'roll') {
                toast.success('New URL made. The old one stopped working.');
                await onChanged();
            }
        } catch (e) {
            toast.error(errorMessage(e, 'Could not load the URL.'));
        } finally {
            setBusy(false);
            setConfirm(false);
        }
    }

    return (
        <section aria-labelledby={`url-heading-${hook.id}`} className="flex flex-col gap-3.5 rounded-[14px] border border-line bg-panel p-5">
            <h2 id={`url-heading-${hook.id}`} className="m-0 text-[17px] font-semibold">
                Webhook URL
            </h2>
            <p className="m-0 text-sm text-dim">
                Made by this server for “{hook.name}”. Paste it where the other service asks for a webhook URL (in GitHub:
                the repository's Settings › Webhooks); each <span className="font-mono text-ink-soft">POST</span> to it starts a
                session. Keep it secret: the URL is the password.
            </p>
            <SecretValue masked={hook.url_hint} load={() => void reveal('url')} busy={busy} what="URL" value={url} onValue={setUrl} />
            {hook.project_missing && (
                <p role="alert" className="m-0 text-sm text-bad">
                    {hook.project} is no longer on {hook.machine ?? 'the computer'}. Edit the webhook to pick a project.
                </p>
            )}
            {confirm ? (
                <div role="alert" className="flex flex-col gap-3 rounded-[10px] border border-bad-line bg-bad-bg p-3">
                    <p className="m-0 text-sm text-ink">
                        Make a new URL? The old one stops working right away, so update whatever calls it.
                    </p>
                    <div className="flex flex-wrap gap-2">
                        <Button variant="danger" disabled={busy} onClick={() => void reveal('roll')}>
                            Make new URL
                        </Button>
                        <Button variant="outline" onClick={() => setConfirm(false)}>
                            Cancel
                        </Button>
                    </div>
                </div>
            ) : (
                <div>
                    <Button variant="outline" onClick={() => setConfirm(true)}>
                        New URL
                    </Button>
                </div>
            )}
        </section>
    );
}

function Calls({ calls, onRefresh }: { calls: IncomingCall[]; onRefresh: () => void }) {
    return (
        <section aria-labelledby="calls-heading" className="overflow-hidden rounded-[14px] border border-line bg-panel">
            <div className="flex items-center justify-between gap-3 border-b border-line px-5 py-[18px]">
                <h2 id="calls-heading" className="m-0 text-[17px] font-semibold">
                    Recent calls
                </h2>
                <button
                    type="button"
                    onClick={onRefresh}
                    className="cursor-pointer rounded-md px-2 py-1 text-[13px] text-dim hover:bg-raised hover:text-ink"
                >
                    Refresh
                </button>
            </div>
            {calls.length === 0 ? (
                <p className="m-0 px-5 py-6 text-dim">No calls yet. Reveal the URL and call it from the service that should start sessions.</p>
            ) : (
                <div className="relative overflow-x-auto">
                    <table className="w-full min-w-[520px] border-collapse text-sm">
                        <thead>
                            <tr className="text-left text-[13px] text-dim">
                                <th scope="col" className="px-5 py-2.5 font-medium">Result</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">From</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">When</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">
                                    <span className="sr-only">Session</span>
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {calls.map((c) => {
                                const s = CALL[c.status];

                                return (
                                    <tr key={c.id} className="border-t border-line-soft align-top">
                                        <td className="px-5 py-3">
                                            <Pill tone={s.tone}>{s.label}</Pill>
                                            {c.error && <div className="mt-1.5 text-xs text-dim">{c.error}</div>}
                                        </td>
                                        <td className="px-5 py-3 font-mono text-[13px] text-dim">{c.ip ?? '—'}</td>
                                        <td className="px-5 py-3 text-dim">{timeAgo(c.created_at)}</td>
                                        <td className="px-5 py-3 text-right">
                                            {c.session_id && (
                                                <Link to={`/remote?session=${c.session_id}`} className="text-[13px] text-amber-soft hover:underline">
                                                    Open session
                                                </Link>
                                            )}
                                        </td>
                                    </tr>
                                );
                            })}
                        </tbody>
                    </table>
                </div>
            )}
        </section>
    );
}
