import { useState, type FormEvent } from 'react';
import { useAppState } from '@/app-state';
import { Button } from '@/components/el/button';
import { AuthLayout, Field, FormError, inputClass } from '@/layouts/AuthLayout';
import { api, errorMessage } from '@/lib/api';

export default function Login() {
    const { refresh, state } = useAppState();
    const [password, setPassword] = useState('');
    const [code, setCode] = useState('');
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    async function submit(e: FormEvent) {
        e.preventDefault();
        setBusy(true);
        setError(null);
        try {
            await api.post('/api/login', { password, totp_code: code.replace(/\s/g, '') });
            await refresh();
        } catch (err) {
            setError(errorMessage(err, 'Could not sign in.'));
            setCode('');
            setBusy(false);
        }
    }

    return (
        <AuthLayout title="Sign in" description={`To ${state.server_name || 'your Termlab server'}.`}>
            <form onSubmit={submit} className="flex flex-col gap-4">
                <Field id="password" label="Password">
                    <input
                        id="password"
                        type="password"
                        autoComplete="current-password"
                        required
                        autoFocus
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        className={inputClass}
                    />
                </Field>
                <Field id="code" label="Authenticator code" hint="The 6-digit code from your authenticator app.">
                    <input
                        id="code"
                        inputMode="numeric"
                        autoComplete="one-time-code"
                        pattern="[0-9 ]{6,7}"
                        maxLength={7}
                        required
                        value={code}
                        onChange={(e) => setCode(e.target.value)}
                        className={`${inputClass} font-mono tracking-[0.3em]`}
                    />
                </Field>
                <FormError message={error} />
                <Button type="submit" size="lg" disabled={busy}>
                    {busy ? 'Signing in…' : 'Sign in'}
                </Button>
            </form>
            <p className="m-0 text-[13px] text-dim">
                Forgot your password or lost your authenticator? On the server, run{' '}
                <code className="font-mono text-ink-soft">termlab-server reset-password</code> or{' '}
                <code className="font-mono text-ink-soft">reset-totp</code>.
            </p>
        </AuthLayout>
    );
}
