import { useState, type FormEvent } from 'react';
import { useAppState } from '@/app-state';
import { Button } from '@/components/el/button';
import { AuthLayout, Field, FormError, TotpQr, inputClass } from '@/layouts/AuthLayout';
import { api, errorMessage } from '@/lib/api';
import type { TotpEnrolment } from '@/types';

/**
 * First run: proves you can read the server's log (the setup token), then
 * creates the owner with a password and an authenticator app.
 */
export default function Setup() {
    const { refresh } = useAppState();
    const [token, setToken] = useState('');
    const [password, setPassword] = useState('');
    const [confirm, setConfirm] = useState('');
    const [enrolment, setEnrolment] = useState<TotpEnrolment | null>(null);
    const [code, setCode] = useState('');
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    async function start(e: FormEvent) {
        e.preventDefault();
        setError(null);
        if (password.length < 12) {
            setError('Use at least 12 characters.');

            return;
        }
        if (password !== confirm) {
            setError('The two passwords are different.');

            return;
        }
        setBusy(true);
        try {
            setEnrolment(await api.post<TotpEnrolment>('/api/setup/start', { setup_token: token.trim() }));
        } catch (err) {
            setError(errorMessage(err, 'Could not start setup.'));
        } finally {
            setBusy(false);
        }
    }

    async function finish(e: FormEvent) {
        e.preventDefault();
        setBusy(true);
        setError(null);
        try {
            await api.post('/api/setup/finish', {
                setup_token: token.trim(),
                password,
                totp_code: code.replace(/\s/g, ''),
            });
            await refresh();
        } catch (err) {
            setError(errorMessage(err, 'That code didn’t work. Try the next one.'));
            setCode('');
            setBusy(false);
        }
    }

    if (enrolment) {
        return (
            <AuthLayout
                title="Add an authenticator"
                description="Every sign-in needs your password and a code from this app, so a leaked password alone can't reach your computers."
            >
                <TotpQr svg={enrolment.qr_svg} secret={enrolment.totp_secret} url={enrolment.otpauth_url} />
                <form onSubmit={finish} className="flex flex-col gap-4">
                    <Field id="code" label="Code from the app">
                        <input
                            id="code"
                            inputMode="numeric"
                            autoComplete="one-time-code"
                            pattern="[0-9 ]{6,7}"
                            maxLength={7}
                            required
                            autoFocus
                            value={code}
                            onChange={(e) => setCode(e.target.value)}
                            className={`${inputClass} font-mono tracking-[0.3em]`}
                        />
                    </Field>
                    <FormError message={error} />
                    <Button type="submit" size="lg" disabled={busy}>
                        {busy ? 'Finishing…' : 'Finish setup'}
                    </Button>
                </form>
            </AuthLayout>
        );
    }

    return (
        <AuthLayout
            title="Set up your server"
            description="This server is yours alone: one owner account reaches your Termlab computers from a browser or phone."
        >
            <form onSubmit={start} className="flex flex-col gap-4">
                <Field
                    id="token"
                    label="Setup token"
                    hint={
                        <>
                            Printed in the server’s log when it started, e.g.{' '}
                            <code className="font-mono">docker logs termlab-server</code> or{' '}
                            <code className="font-mono">journalctl -u termlab-server</code>.
                        </>
                    }
                >
                    <input
                        id="token"
                        autoComplete="off"
                        spellCheck={false}
                        required
                        autoFocus
                        value={token}
                        onChange={(e) => setToken(e.target.value)}
                        className={`${inputClass} font-mono`}
                    />
                </Field>
                <Field id="password" label="Password" hint="At least 12 characters.">
                    <input
                        id="password"
                        type="password"
                        autoComplete="new-password"
                        minLength={12}
                        required
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        className={inputClass}
                    />
                </Field>
                <Field id="confirm" label="Password again">
                    <input
                        id="confirm"
                        type="password"
                        autoComplete="new-password"
                        required
                        value={confirm}
                        onChange={(e) => setConfirm(e.target.value)}
                        className={inputClass}
                    />
                </Field>
                <FormError message={error} />
                <Button type="submit" size="lg" disabled={busy}>
                    {busy ? 'Checking…' : 'Continue'}
                </Button>
            </form>
        </AuthLayout>
    );
}
