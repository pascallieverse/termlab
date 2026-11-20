import type { ReactNode } from 'react';
import { Button } from '@/components/el/button';
import { Segmented } from '@/components/el/segmented';
import {
    DEFAULT_APPEARANCE,
    FONT_SIZE_RANGE,
    LINE_HEIGHTS,
    TERMINAL_THEMES,
    setAppearance,
    useAppearance,
    type Appearance,
} from '@/lib/appearance';
import { cn } from '@/lib/utils';

// Settings › Appearance: the desktop app's Appearance and Terminal sections, for the
// terminals in this browser. Every choice applies at once and stays in this browser.

const SWATCH_KEYS = ['red', 'green', 'yellow', 'blue', 'magenta', 'cyan'] as const;

export function AppearanceSection() {
    const a = useAppearance();
    const isDefault = (Object.keys(DEFAULT_APPEARANCE) as (keyof Appearance)[]).every((k) => a[k] === DEFAULT_APPEARANCE[k]);

    return (
        <>
            <TerminalPreview />

            <Group title="Theme" note="A theme colours this page and its terminals together, as in Termlab. Agents that draw with their own exact colours (Claude Code's default themes do) keep those for that text.">
                <div className="settings-block">
                    <ThemePicker />
                </div>
            </Group>

            <Group title="Text">
                <Row title="Font size" labelId="ap-font" desc="JetBrains Mono">
                    <div className="stepper" role="group" aria-labelledby="ap-font">
                        <button
                            type="button"
                            className="icon-btn stepper-btn"
                            aria-label="Smaller"
                            disabled={a.terminalFontSize <= FONT_SIZE_RANGE.min}
                            onClick={() => setAppearance({ terminalFontSize: a.terminalFontSize - 1 })}
                        >
                            <span aria-hidden="true">−</span>
                        </button>
                        <output className="font-mono" aria-live="polite">
                            {a.terminalFontSize}px
                        </output>
                        <button
                            type="button"
                            className="icon-btn stepper-btn"
                            aria-label="Larger"
                            disabled={a.terminalFontSize >= FONT_SIZE_RANGE.max}
                            onClick={() => setAppearance({ terminalFontSize: a.terminalFontSize + 1 })}
                        >
                            <span aria-hidden="true">+</span>
                        </button>
                    </div>
                </Row>
                <Row title="Line spacing" labelId="ap-lh">
                    <Segmented
                        label="Line spacing"
                        size="sm"
                        value={String(a.terminalLineHeight)}
                        options={LINE_HEIGHTS.map((l) => ({ value: String(l.value), label: l.label }))}
                        onChange={(v) => setAppearance({ terminalLineHeight: Number(v) })}
                    />
                </Row>
            </Group>

            <Group
                title="Terminal size"
                note="A session has one terminal, so one size. Whichever screen you're using sets it: Termlab takes it back when you open the session there or resize its window."
            >
                <Row
                    title="Size"
                    labelId="ap-size"
                    desc={
                        a.terminalSize === 'browser'
                            ? "While you watch a session here, it takes this browser's size and the agent redraws for it. Termlab's window shows the smaller screen meanwhile."
                            : "Sessions keep the size of Termlab's window; this browser shows them as they are."
                    }
                >
                    <Segmented
                        label="Terminal size"
                        size="sm"
                        value={a.terminalSize}
                        options={[
                            { value: 'browser', label: 'Fit this browser' },
                            { value: 'app', label: "Termlab's size" },
                        ]}
                        onChange={(v) => setAppearance({ terminalSize: v as 'browser' | 'app' })}
                    />
                </Row>
                {a.terminalSize === 'app' && (
                    <SwitchRow
                        title="Fit the session's width"
                        desc="When a session's terminal is wider than this window, the text gets smaller (down to 5px) so lines don't run off the side."
                        checked={a.fitWidth}
                        onChange={(on) => setAppearance({ fitWidth: on })}
                    />
                )}
            </Group>

            <Group title="Cursor">
                <SwitchRow title="Blink" checked={a.cursorBlink} onChange={(on) => setAppearance({ cursorBlink: on })} />
            </Group>

            <div>
                <Button variant="default" disabled={isDefault} onClick={() => setAppearance(DEFAULT_APPEARANCE)}>
                    Reset appearance
                </Button>
            </div>
        </>
    );
}

export function Group({ title, note, children }: { title: string; note?: ReactNode; children: ReactNode }) {
    return (
        <section className="settings-group">
            <h3 className="section-heading">{title}</h3>
            <div className="settings-card">{children}</div>
            {note && <p className="settings-note">{note}</p>}
        </section>
    );
}

export function Row({ title, desc, labelId, children }: { title: ReactNode; desc?: ReactNode; labelId?: string; children?: ReactNode }) {
    return (
        <div className="settings-row">
            <div className="settings-row-text">
                <span className="settings-row-title" id={labelId}>
                    {title}
                </span>
                {desc && <span className="text-xs text-dim">{desc}</span>}
            </div>
            {children && <div className="settings-row-control">{children}</div>}
        </div>
    );
}

function SwitchRow({ title, desc, checked, onChange }: { title: string; desc?: ReactNode; checked: boolean; onChange(on: boolean): void }) {
    return (
        <label className="settings-row clickable">
            <span className="settings-row-text">
                <span className="settings-row-title">{title}</span>
                {desc && <span className="text-xs text-dim">{desc}</span>}
            </span>
            <input type="checkbox" role="switch" className="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
        </label>
    );
}

/** One choice per desktop theme; each swatch is a small terminal in its colours. */
function ThemePicker() {
    const { terminalTheme } = useAppearance();

    return (
        <div role="radiogroup" aria-label="Theme" className="theme-grid">
            {Object.entries(TERMINAL_THEMES).map(([id, t]) => {
                const on = id === terminalTheme;

                return (
                    <button
                        key={id}
                        type="button"
                        role="radio"
                        aria-checked={on}
                        className={cn('theme-option', on && 'on')}
                        onClick={() => setAppearance({ terminalTheme: id })}
                    >
                        <span className="theme-swatch font-mono" aria-hidden="true" style={{ background: t.theme.background, color: t.theme.foreground }}>
                            <span>
                                <span style={{ color: t.theme.green }}>❯</span> npm test
                            </span>
                            <span className="term-swatch-colors">
                                {SWATCH_KEYS.map((k) => (
                                    <span key={k} style={{ background: t.theme[k] }} />
                                ))}
                            </span>
                        </span>
                        <span className="theme-name">{t.name}</span>
                        <span className="text-xs text-dim">{t.description}</span>
                    </button>
                );
            })}
        </div>
    );
}

/** A few lines drawn like a terminal, with the chosen colours, size, spacing and cursor. */
function TerminalPreview() {
    const a = useAppearance();
    const t = (TERMINAL_THEMES[a.terminalTheme] ?? TERMINAL_THEMES.default).theme;

    return (
        <div
            className="term-preview font-mono"
            role="img"
            aria-label="Preview of the terminal"
            style={{ background: t.background, color: t.foreground, fontSize: a.terminalFontSize, lineHeight: a.terminalLineHeight }}
        >
            <div>
                <span style={{ color: t.green }}>~/code/billing-api</span> <span style={{ color: t.blue }}>main</span>
            </div>
            <div>
                <span style={{ color: t.magenta }}>❯</span> npm test
            </div>
            <div>
                <span style={{ color: t.green }}>✓</span> 42 passed <span style={{ color: t.brightBlack }}>(1.8s)</span>
            </div>
            <div>
                <span style={{ color: t.yellow }}>!</span> 1 skipped · <span style={{ color: t.red }}>0 failed</span>
            </div>
            <div>
                <span style={{ color: t.magenta }}>❯</span>{' '}
                <span className={cn('term-preview-cursor', a.cursorBlink && 'blink')} style={{ background: t.cursor }}>
                    {' '}
                </span>
            </div>
        </div>
    );
}
