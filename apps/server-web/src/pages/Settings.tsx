import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from 'react';
import { useAppState } from '@/app-state';
import { Button } from '@/components/el/button';
import { useToast } from '@/components/el/toast';
import { PushToggle } from '@/components/push-toggle';
import { AppShell, PageHeader } from '@/layouts/AppShell';
import { TotpQr } from '@/layouts/AuthLayout';
import { api, errorMessage } from '@/lib/api';
import { formatDate, timeAgo } from '@/lib/format';
import type { AuditEntry, DeviceSession, TotpEnrolment } from '@/types';

const inputClass = 'min-h-11 w-full rounded-[9px] border border-line-strong bg-bg px-3 text-ink';

function Section({ id, title, description, children }: { id: string; title: string; description?: ReactNode; children: ReactNode }) {
    return (
        <section aria-labelledby={id} className="flex flex-col gap-4 rounded-[14px] border border-line bg-panel p-5">
            <div className="flex flex-col gap-1">
                <h2 id={id} className="m-0 text-[17px] font-semibold">
                    {title}
                </h2>
                {description && <p className="m-0 text-sm text-dim">{description}</p>}
            </div>
            {children}
        </section>
    );
}

function Input(props: React.InputHTMLAttributes<HTMLInputElement> & { label: string; id: string }) {
    const { label, ...rest } = props;

    return (
        <div className="flex flex-col gap-1.5">
            <label htmlFor={rest.id} className="text-sm font-medium">
                {label}
            </label>
            <input {...rest} className={inputClass} />
        </div>
    );
}

export default function Settings() {
    const { state } = useAppState();

    return (
        <AppShell>
            <PageHeader title="Settings" description={`${state.server_name || 'Termlab Server'} · version ${state.version}`} />
            <div className="grid grid-cols-[repeat(auto-fit,minmax(min(380px,100%),1fr))] items-start gap-5">
                <PasswordSection />
                <TotpSection />
                <NotificationsSection />
                <DevicesSection />
            </div>
            <AuditSection />
            <SignOut />
        </AppShell>
    );
}

function PasswordSection() {
    const toast = useToast();
    const [current, setCurrent] = useState('');
    const [next, setNext] = useState('');
    const [code, setCode] = useState('');
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    async function submit(e: FormEvent) {
        e.preventDefault();
        if (next.length < 12) {
            setError('Use at least 12 characters.');

            return;
        }
        setBusy(true);
        setError(null);
        try {
            await api.post('/api/account/password', {
                current_password: current,
                new_password: next,
                totp_code: code.replace(/\s/g, ''),
            });
            setCurrent('');
            setNext('');
            setCode('');
            toast.success('Password changed. Other devices were signed out.');
        } catch (err) {
            setError(errorMessage(err, 'Could not change the password.'));
        } finally {
            setBusy(false);
        }
    }

    return (
        <Section id="password-heading" title="Password" description="Changing it signs out every other device.">
            <form onSubmit={submit} className="flex flex-col gap-3">
                <Input id="current-password" label="Current password" type="password" autoComplete="current-password" required value={current} onChange={(e) => setCurrent(e.target.value)} />
                <Input id="new-password" label="New password" type="password" autoComplete="new-password" minLength={12} required value={next} onChange={(e) => setNext(e.target.value)} />
                <Input id="password-code" label="Authenticator code" inputMode="numeric" autoComplete="one-time-code" maxLength={7} required value={code} onChange={(e) => setCode(e.target.value)} />
                {error && (
                    <p role="alert" className="m-0 text-sm text-bad">
                        {error}
                    </p>
                )}
                <Button type="submit" disabled={busy} className="self-start">
                    Change password
                </Button>
            </form>
        </Section>
    );
}

function TotpSection() {
    const toast = useToast();
    const [enrolment, setEnrolment] = useState<TotpEnrolment | null>(null);
    const [password, setPassword] = useState('');
    const [code, setCode] = useState('');
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    async function start() {
        setBusy(true);
        setError(null);
        try {
            setEnrolment(await api.post<TotpEnrolment>('/api/account/totp/start'));
        } catch (err) {
            setError(errorMessage(err, 'Could not start.'));
        } finally {
            setBusy(false);
        }
    }

    async function finish(e: FormEvent) {
        e.preventDefault();
        setBusy(true);
        setError(null);
        try {
            await api.post('/api/account/totp/finish', { password, totp_code: code.replace(/\s/g, '') });
            setEnrolment(null);
            setPassword('');
            setCode('');
            toast.success('Authenticator changed. Use the new app from now on.');
        } catch (err) {
            setError(errorMessage(err, 'That didn’t work. Check the password and try the next code.'));
            setCode('');
        } finally {
            setBusy(false);
        }
    }

    return (
        <Section
            id="totp-heading"
            title="Authenticator app"
            description="Every sign-in needs a code from it. Move it to a new phone here; the old app stops working once you confirm."
        >
            {enrolment ? (
                <form onSubmit={finish} className="flex flex-col gap-3">
                    <TotpQr svg={enrolment.qr_svg} secret={enrolment.totp_secret} url={enrolment.otpauth_url} />
                    <Input id="totp-password" label="Password" type="password" autoComplete="current-password" required value={password} onChange={(e) => setPassword(e.target.value)} />
                    <Input id="totp-code" label="Code from the new app" inputMode="numeric" autoComplete="one-time-code" maxLength={7} required value={code} onChange={(e) => setCode(e.target.value)} />
                    {error && (
                        <p role="alert" className="m-0 text-sm text-bad">
                            {error}
                        </p>
                    )}
                    <div className="flex flex-wrap gap-2">
                        <Button type="submit" disabled={busy}>
                            Switch to this app
                        </Button>
                        <Button variant="outline" onClick={() => setEnrolment(null)}>
                            Cancel
                        </Button>
                    </div>
                </form>
            ) : (
                <>
                    {error && (
                        <p role="alert" className="m-0 text-sm text-bad">
                            {error}
                        </p>
                    )}
                    <Button variant="outline" disabled={busy} onClick={() => void start()} className="self-start">
                        Set up a new authenticator
                    </Button>
                </>
            )}
        </Section>
    );
}

type PushDevice = { id: number; label: string; created_at: string };

function NotificationsSection() {
    const toast = useToast();
    const [devices, setDevices] = useState<PushDevice[]>([]);

    const load = useCallback(async () => {
        try {
            const data = await api.get<{ public_key: string; subscriptions: PushDevice[] }>('/api/push');
            setDevices(data.subscriptions);
        } catch {
            setDevices([]);
        }
    }, []);

    useEffect(() => {
        void load();
    }, [load]);

    return (
        <Section
            id="push-heading"
            title="Notifications"
            description="A notification on your phone or computer when a session needs you or fails."
        >
            <PushToggle onChange={() => void load()} />
            {devices.length > 0 && (
                <ul className="m-0 flex list-none flex-col divide-y divide-line-soft rounded-[10px] border border-line p-0">
                    {devices.map((d) => (
                        <li key={d.id} className="flex items-center justify-between gap-3 px-3 py-2.5">
                            <span className="flex min-w-0 flex-col">
                                <span className="truncate text-sm">{d.label}</span>
                                <span className="text-xs text-dim">since {formatDate(d.created_at)}</span>
                            </span>
                            <Button
                                variant="ghost"
                                size="sm"
                                aria-label={`Remove ${d.label}`}
                                onClick={async () => {
                                    try {
                                        await api.del(`/api/push/${d.id}`);
                                        await load();
                                    } catch (e) {
                                        toast.error(errorMessage(e, 'Could not remove the device.'));
                                    }
                                }}
                            >
                                Remove
                            </Button>
                        </li>
                    ))}
                </ul>
            )}
            {devices.length > 0 && (
                <Button
                    variant="outline"
                    className="self-start"
                    onClick={async () => {
                        try {
                            await api.post('/api/push/test');
                            toast.success('Test notification sent.');
                        } catch (e) {
                            toast.error(errorMessage(e, 'Could not send a test notification.'));
                        }
                    }}
                >
                    Send a test notification
                </Button>
            )}
        </Section>
    );
}

function DevicesSection() {
    const toast = useToast();
    const [devices, setDevices] = useState<DeviceSession[] | null>(null);

    const load = useCallback(async () => {
        try {
            setDevices(await api.get<DeviceSession[]>('/api/account/sessions'));
        } catch {
            setDevices([]);
        }
    }, []);

    useEffect(() => {
        void load();
    }, [load]);

    return (
        <Section id="devices-heading" title="Signed-in devices" description="Browsers and phones signed in to this server.">
            {devices === null ? (
                <p className="m-0 text-sm text-dim">Loading…</p>
            ) : (
                <ul className="m-0 flex list-none flex-col divide-y divide-line-soft rounded-[10px] border border-line p-0">
                    {devices.map((d) => (
                        <li key={d.id} className="flex items-center justify-between gap-3 px-3 py-2.5">
                            <span className="flex min-w-0 flex-col">
                                <span className="truncate text-sm">
                                    {d.user_agent ? shortAgent(d.user_agent) : 'Unknown browser'}
                                    {d.current && <span className="ml-2 text-xs text-ok">this device</span>}
                                </span>
                                <span className="text-xs text-dim">
                                    {d.ip ?? 'unknown address'} · active {timeAgo(d.last_seen_at ?? d.created_at)}
                                </span>
                            </span>
                            {!d.current && (
                                <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={async () => {
                                        try {
                                            await api.del(`/api/account/sessions/${d.id}`);
                                            await load();
                                            toast.success('Signed out that device.');
                                        } catch (e) {
                                            toast.error(errorMessage(e, 'Could not sign it out.'));
                                        }
                                    }}
                                >
                                    Sign out
                                </Button>
                            )}
                        </li>
                    ))}
                </ul>
            )}
        </Section>
    );
}

function shortAgent(ua: string): string {
    const browser = /Edg\//.test(ua) ? 'Edge' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : null;
    const os = /iPhone/.test(ua) ? 'iPhone' : /iPad/.test(ua) ? 'iPad' : /Android/.test(ua) ? 'Android' : /Mac OS X/.test(ua) ? 'Mac' : /Windows/.test(ua) ? 'Windows' : /Linux/.test(ua) ? 'Linux' : null;

    return browser && os ? `${browser} on ${os}` : ua.slice(0, 60);
}

// The actions termlab-server records (crates/termlab-server: notify::audit calls).
const ACTION_LABEL: Record<string, string> = {
    'account.created': 'Created the owner account',
    'account.signed_in': 'Signed in',
    'account.sign_in_failed': 'Failed sign-in',
    'account.password_changed': 'Changed the password',
    'account.totp_changed': 'Changed the authenticator',
    'account.signed_out_device': 'Signed out a device',
    'machine.pairing_code': 'Made a pairing code',
    'machine.paired': 'Paired a machine',
    'machine.unpaired': 'A machine unlinked itself',
    'machine.removed': 'Removed a machine',
    'approval.allow_once': 'Allowed once',
    'approval.allow_always': 'Always allowed',
    'approval.deny': 'Denied',
    'session.message': 'Sent a message',
    'session.stop': 'Stopped a session',
    'session.hold': 'Put a session on hold',
    'webhook.created': 'Added a webhook',
    'webhook.deleted': 'Deleted a webhook',
    'webhook.secret_rolled': 'Rolled a webhook secret',
    'push.device_added': 'Turned on notifications for a device',
};

function AuditSection() {
    const [entries, setEntries] = useState<AuditEntry[]>([]);
    const [next, setNext] = useState<number | null>(null);
    const [loading, setLoading] = useState(true);

    const load = useCallback(async (before: number | null) => {
        setLoading(true);
        try {
            const data = await api.get<{ entries: AuditEntry[]; next: number | null }>(
                before === null ? '/api/audit' : `/api/audit?before=${before}`,
            );
            setEntries((list) => (before === null ? data.entries : [...list, ...data.entries]));
            setNext(data.next);
        } catch {
            // Leave what's shown.
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        void load(null);
    }, [load]);

    return (
        <section aria-labelledby="audit-heading" className="overflow-hidden rounded-[14px] border border-line bg-panel">
            <div className="flex flex-col gap-1 border-b border-line px-5 py-[18px]">
                <h2 id="audit-heading" className="m-0 text-[17px] font-semibold">
                    Audit log
                </h2>
                <p className="m-0 text-sm text-dim">Sign-ins and everything done to your sessions from this server.</p>
            </div>
            {entries.length === 0 ? (
                <p className="m-0 px-5 py-6 text-dim">{loading ? 'Loading…' : 'Nothing yet.'}</p>
            ) : (
                <div className="relative overflow-x-auto">
                    <table className="w-full min-w-[560px] border-collapse text-sm">
                        <thead>
                            <tr className="text-left text-[13px] text-dim">
                                <th scope="col" className="px-5 py-2.5 font-medium">What</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">Session</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">From</th>
                                <th scope="col" className="px-5 py-2.5 font-medium">When</th>
                            </tr>
                        </thead>
                        <tbody>
                            {entries.map((e) => (
                                <tr key={e.id} className="border-t border-line-soft align-top">
                                    <td className="px-5 py-3">
                                        <span className="flex flex-col">
                                            <span>{ACTION_LABEL[e.action] ?? e.action}</span>
                                            {e.detail && <span className="font-mono text-xs break-all text-dim">{e.detail}</span>}
                                        </span>
                                    </td>
                                    <td className="px-5 py-3 text-ink-soft">
                                        {e.session_title ? `${e.session_title}${e.machine ? ` · ${e.machine}` : ''}` : (e.machine ?? '—')}
                                    </td>
                                    <td className="px-5 py-3 font-mono text-xs text-dim">{e.ip ?? '—'}</td>
                                    <td className="px-5 py-3 text-dim">{timeAgo(e.created_at)}</td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
            {next !== null && (
                <div className="border-t border-line-soft px-5 py-3">
                    <Button variant="outline" size="sm" disabled={loading} onClick={() => void load(next)}>
                        Load more
                    </Button>
                </div>
            )}
        </section>
    );
}

function SignOut() {
    const { refresh } = useAppState();
    const toast = useToast();

    return (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-[14px] border border-line bg-panel px-5 py-4">
            <span className="text-sm text-dim">Sign out of this browser.</span>
            <Button
                variant="outline"
                onClick={async () => {
                    try {
                        await api.post('/api/logout');
                    } catch (e) {
                        toast.error(errorMessage(e, 'Could not sign out.'));
                    }
                    await refresh();
                }}
            >
                Sign out
            </Button>
        </div>
    );
}
