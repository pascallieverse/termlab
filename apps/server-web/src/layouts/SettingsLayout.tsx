import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Link, useNavigate } from 'react-router-dom';
import { useAppState } from '@/app-state';
import { cn } from '@/lib/utils';
import { Icon, type IconName } from './nav';

export type SettingsSection = { id: string; label: string; icon: IconName; desc: string };

/** Where a section puts its main button (Pair a machine, Add endpoint): the section's header. */
const ActionsSlot = createContext<HTMLElement | null>(null);

export function HeadActions({ children }: { children: ReactNode }) {
    const el = useContext(ActionsSlot);

    return el ? createPortal(children, el) : null;
}

/**
 * The dialog's frame: a dimmed backdrop over the sessions; Escape or a click outside closes
 * it. On phones it fills the screen.
 */
function Overlay({ children, label, sheet }: { children: ReactNode; label: string; sheet?: boolean }) {
    const navigate = useNavigate();
    const box = useRef<HTMLDivElement>(null);
    const close = () => navigate('/remote');

    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            // A section's own dialog (pairing, webhooks) handles its Escape first.
            if (e.key === 'Escape' && !e.defaultPrevented && !document.querySelector('[role="dialog"] [role="dialog"], dialog[open]')) close();
        };
        window.addEventListener('keydown', onKey);
        const before = document.activeElement as HTMLElement | null;
        box.current?.focus();
        const body = document.body.style.overflow;
        document.body.style.overflow = 'hidden';

        return () => {
            window.removeEventListener('keydown', onKey);
            document.body.style.overflow = body;
            before?.focus?.();
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    return (
        <div
            className={cn('settings-overlay', sheet && 'sheet')}
            onMouseDown={(e) => {
                if (e.target === e.currentTarget) close();
            }}
        >
            <div ref={box} role="dialog" aria-modal="true" aria-label={label} tabIndex={-1} className="settings-dialog">
                {children}
            </div>
        </div>
    );
}

/**
 * Settings, laid out like the desktop app's Settings dialog: the sections on the left, the
 * open one on the right with its title, description and close button.
 */
export function SettingsLayout({
    sections,
    current,
    children,
}: {
    sections: SettingsSection[];
    current: SettingsSection;
    children: ReactNode;
}) {
    const { state } = useAppState();
    const [slot, setSlot] = useState<HTMLElement | null>(null);

    return (
        <Overlay label="Settings">
                <div className="settings-layout">
                    <nav className="settings-nav" aria-labelledby="st-title">
                        <h1 id="st-title" className="settings-nav-title">
                            Settings
                        </h1>
                        <div className="settings-tabs">
                            {sections.map((s) => (
                                <Link
                                    key={s.id}
                                    to={`/settings/${s.id}`}
                                    aria-current={s.id === current.id ? 'page' : undefined}
                                    className={cn('settings-tab', s.id === current.id && 'on')}
                                >
                                    <Icon name={s.icon} />
                                    <span className="settings-tab-label">{s.label}</span>
                                </Link>
                            ))}
                        </div>
                        <div className="settings-nav-foot font-mono">Termlab Server {state.version}</div>
                    </nav>

                    <main className="settings-main" aria-labelledby="st-section">
                        <div className="settings-main-head">
                            <div className="settings-main-title">
                                <h2 id="st-section">{current.label}</h2>
                                <p>{current.desc}</p>
                            </div>
                            <div ref={setSlot} className="settings-head-actions" />
                            <Link to="/remote" className="icon-btn" aria-label="Close settings" title="Close settings">
                                <Icon name="close" />
                            </Link>
                        </div>
                        <ActionsSlot.Provider value={slot}>
                            <div key={current.id} className="settings-content">
                                {children}
                            </div>
                        </ActionsSlot.Provider>
                    </main>
                </div>
        </Overlay>
    );
}

/** Phones, /settings: every section as a row to tap into. */
export function SettingsPhoneList({ sections }: { sections: SettingsSection[] }) {
    const { state } = useAppState();

    return (
        <Overlay label="Settings" sheet>
            <header className="settings-phone-head">
                <h1 id="st-title" className="m-0 min-w-0 flex-1 truncate pl-2 text-base font-semibold">
                    Settings
                </h1>
                <Link to="/remote" className="icon-btn size-11" aria-label="Close settings">
                    <Icon name="close" size={18} />
                </Link>
            </header>
            <main className="settings-phone-list" aria-labelledby="st-title">
                <nav aria-labelledby="st-title" className="settings-card">
                    {sections.map((s) => (
                        <Link key={s.id} to={`/settings/${s.id}`} className="settings-phone-row">
                            <span className="settings-phone-icon">
                                <Icon name={s.icon} size={18} />
                            </span>
                            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                                <span className="font-medium text-ink">{s.label}</span>
                                <span className="truncate text-xs text-dim">{s.desc}</span>
                            </span>
                            <span className="text-faint">
                                <Icon name="chevron" />
                            </span>
                        </Link>
                    ))}
                </nav>
                <p className="m-0 text-center font-mono text-xs text-faint">Termlab Server {state.version}</p>
            </main>
        </Overlay>
    );
}

/** Phones, /settings/<section>: a back button, the title, then the section. */
export function SettingsPhoneSection({ current, children }: { current: SettingsSection; children: ReactNode }) {
    const [slot, setSlot] = useState<HTMLElement | null>(null);

    return (
        <Overlay label={current.label} sheet>
            <header className="settings-phone-head">
                <Link to="/settings" className="icon-btn size-11" aria-label="Back to settings">
                    <Icon name="back" size={18} />
                </Link>
                <h1 className="m-0 min-w-0 flex-1 truncate text-base font-semibold">{current.label}</h1>
                <div ref={setSlot} className="settings-head-actions" />
            </header>
            <main className="settings-phone-content">
                <p className="m-0 text-[13px] leading-relaxed text-dim">{current.desc}</p>
                <ActionsSlot.Provider value={slot}>{children}</ActionsSlot.Provider>
            </main>
        </Overlay>
    );
}

/** A row of quiet text above a section's content (e.g. "2 of 3 online"). */
export function SectionNote({ children }: { children: ReactNode }) {
    return <p className="m-0 text-[13px] text-dim">{children}</p>;
}
