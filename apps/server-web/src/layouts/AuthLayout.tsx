import type { ReactNode } from 'react';
import { LogoMark } from '@/components/el/logo';

/** Sign-in and first-run setup: one centred column. */
export function AuthLayout({ title, description, children }: { title: string; description?: ReactNode; children: ReactNode }) {
    return (
        <div className="flex min-h-dvh flex-col bg-bg px-6 py-8 text-base leading-[1.55] text-ink">
            <div className="flex items-center gap-2.5">
                <LogoMark />
                <span className="font-display text-[19px] font-bold">
                    Termlab <span className="text-amber">Server</span>
                </span>
            </div>
            <main className="m-auto flex w-full max-w-[420px] flex-col gap-6 py-14">
                <div className="flex flex-col gap-2">
                    <h1 className="m-0 font-display text-4xl tracking-[-0.02em]">{title}</h1>
                    {description && <p className="m-0 text-dim">{description}</p>}
                </div>
                {children}
            </main>
        </div>
    );
}

export function Field({
    id,
    label,
    hint,
    children,
}: {
    id: string;
    label: string;
    hint?: ReactNode;
    children: ReactNode;
}) {
    return (
        <div className="flex flex-col gap-1.5">
            <label htmlFor={id} className="text-sm font-medium">
                {label}
            </label>
            {children}
            {hint && <span className="text-[13px] text-dim">{hint}</span>}
        </div>
    );
}

export function FormError({ message }: { message: string | null }) {
    if (!message) {
        return null;
    }

    return (
        <p role="alert" className="m-0 rounded-[10px] border border-bad-line bg-bad-bg px-3 py-2.5 text-sm text-bad">
            {message}
        </p>
    );
}

export const inputClass =
    'min-h-[46px] w-full rounded-[10px] border border-line-strong bg-surface px-3.5 text-ink';

/** The QR code the server rendered, plus the secret for typing it in by hand. */
export function TotpQr({ svg, secret, url }: { svg: string; secret: string; url: string }) {
    const safe = svg.trimStart().startsWith('<svg');

    return (
        <div className="flex flex-col items-center gap-3 rounded-xl border border-line bg-panel p-4">
            {safe && (
                <div
                    role="img"
                    aria-label="QR code for your authenticator app"
                    className="w-48 rounded-lg bg-white p-2 [&_svg]:block [&_svg]:h-auto [&_svg]:w-full"
                    dangerouslySetInnerHTML={{ __html: svg }}
                />
            )}
            <p className="m-0 text-center text-[13px] text-dim">
                Scan it with an authenticator app, or enter this key:
            </p>
            <code className="rounded-md border border-line bg-bg px-2 py-1 font-mono text-sm tracking-wider break-all text-ink-soft">
                {secret}
            </code>
            <a href={url} className="text-[13px]">
                Open in an authenticator app on this device
            </a>
        </div>
    );
}
