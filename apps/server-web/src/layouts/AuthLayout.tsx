import type { ReactNode } from 'react';
import { LogoMark } from '@/components/el/logo';
import { cn } from '@/lib/utils';

/** Sign-in and first-run setup: one centred column. */
export function AuthLayout({ title, description, children }: { title: string; description?: ReactNode; children: ReactNode }) {
    return (
        <div className="flex min-h-dvh flex-col bg-bg text-sm leading-normal text-ink">
            <header className="app-header">
                <span className="brand">
                    <LogoMark />
                    <span>
                        Termlab <span className="brand-sub">Server</span>
                    </span>
                </span>
            </header>
            <main className="m-auto flex w-full max-w-[440px] flex-col gap-5 px-4 py-12">
                <div className="flex flex-col gap-1.5">
                    <h1 className="m-0 text-2xl font-semibold tracking-[-0.01em]">{title}</h1>
                    {description && <p className="m-0 text-dim">{description}</p>}
                </div>
                <div className={cn('flex flex-col gap-4 rounded-[14px] border border-line-strong bg-panel p-[22px]')}>{children}</div>
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
            <label htmlFor={id} className="text-xs font-semibold text-dim">
                {label}
            </label>
            {children}
            {hint && <span className="text-xs text-faint">{hint}</span>}
        </div>
    );
}

export function FormError({ message }: { message: string | null }) {
    if (!message) {
        return null;
    }

    return (
        <p role="alert" className="m-0 rounded-[8px] border border-bad-line bg-bad-bg px-3 py-2.5 text-[13px] text-bad">
            {message}
        </p>
    );
}

export const inputClass =
    'min-h-9 w-full rounded-[8px] border border-line-strong bg-raised px-3 text-ink outline-none focus:border-[#3a4250] max-md:min-h-11';

/** The QR code the server rendered, plus the secret for typing it in by hand. */
export function TotpQr({ svg, secret, url }: { svg: string; secret: string; url: string }) {
    const safe = svg.trimStart().startsWith('<svg');

    return (
        <div className="flex flex-col items-center gap-3 rounded-[10px] border border-line bg-bg p-4">
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
