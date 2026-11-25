import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { TERMINAL_THEMES, useAppearance, type Appearance } from '@/lib/appearance';
import { useLive } from '@/live/LiveProvider';

function decode(base64: string): Uint8Array {
    const bin = atob(base64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) {
        bytes[i] = bin.charCodeAt(i);
    }

    return bytes;
}

function encode(text: string): string {
    const bytes = new TextEncoder().encode(text);
    let bin = '';
    bytes.forEach((b) => (bin += String.fromCharCode(b)));

    return btoa(bin);
}

const FONT = '"JetBrains Mono", Menlo, monospace';
/** The smallest font the fit-to-width option goes down to; below it, scroll sideways. */
const MIN_FIT = { wide: 5, narrow: 6 };

/** Width of one monospace cell per pixel of font size. */
let cellRatio = 0;
function cellWidth(fontSize: number): number {
    if (!cellRatio) {
        const ctx = document.createElement('canvas').getContext('2d');
        if (ctx) {
            ctx.font = `100px ${FONT}`;
            cellRatio = ctx.measureText('W'.repeat(10)).width / 1000;
        }
        cellRatio ||= 0.6;
    }

    return fontSize * cellRatio;
}

function theme(a: Appearance) {
    return (TERMINAL_THEMES[a.terminalTheme] ?? TERMINAL_THEMES.default).theme;
}

/** Keys a phone keyboard doesn't have. */
const KEYS: { label: string; title: string; seq: string }[] = [
    { label: 'Esc', title: 'Escape', seq: '\x1b' },
    { label: 'Tab', title: 'Tab', seq: '\t' },
    { label: '⇧Tab', title: 'Shift+Tab (Claude Code: switch mode)', seq: '\x1b[Z' },
    { label: '↑', title: 'Up', seq: '\x1b[A' },
    { label: '↓', title: 'Down', seq: '\x1b[B' },
    { label: '←', title: 'Left', seq: '\x1b[D' },
    { label: '→', title: 'Right', seq: '\x1b[C' },
    { label: '^C', title: 'Ctrl+C', seq: '\x03' },
    { label: '⏎', title: 'Enter', seq: '\r' },
];

/**
 * Swipes scroll the terminal on touch screens.
 *
 * xterm's own touch scrolling only moves its scrollback: it does nothing while the app tracks
 * the mouse (Claude Code's full-screen view) or is on the alternate screen (vim, less), and it
 * blocks the box around it, which scrolls when the session is bigger than the view. So a swipe
 * scrolls that box first, then goes to the terminal as mouse-wheel turns, which xterm sends to
 * the app when it wants them (or as arrow keys on the alternate screen, as a wheel does).
 */
function swipeToScroll(box: HTMLElement, t: Terminal): () => void {
    let lastX = 0;
    let lastY = 0;
    let axis: 'x' | 'y' | null = null;
    let tracking = false;
    /** Pixels swiped that are not yet a whole wheel turn for the app. */
    let pending = 0;
    let velocity = 0;
    let lastTime = 0;
    let fling = 0;

    const rowHeight = () => {
        const screen = box.querySelector<HTMLElement>('.xterm-screen');
        return screen && t.rows ? screen.offsetHeight / t.rows : 16;
    };

    const wheel = (deltaY: number, deltaMode: number) => {
        const target = box.querySelector<HTMLElement>('.xterm-screen');
        if (!target) return;
        const rect = target.getBoundingClientRect();
        target.dispatchEvent(
            new WheelEvent('wheel', {
                deltaY,
                deltaMode,
                clientX: rect.left + Math.min(rect.width, box.clientWidth) / 2,
                clientY: rect.top + Math.min(rect.height, box.clientHeight) / 2,
                bubbles: true,
                cancelable: true,
            }),
        );
    };

    /** Scrolls by `dy` pixels (positive: toward newer output). */
    const scrollY = (dy: number) => {
        const before = box.scrollTop;
        box.scrollTop = before + dy;
        const rest = dy - (box.scrollTop - before);
        if (Math.abs(rest) < 0.5) return;
        const app = t.modes.mouseTrackingMode !== 'none' || t.buffer.active.type === 'alternate';
        if (!app) {
            wheel(rest, WheelEvent.DOM_DELTA_PIXEL);
            return;
        }
        // Apps scroll a few lines per wheel turn: one turn per few rows swiped keeps the
        // text roughly under the finger (one per row when the turns become arrow keys).
        const row = rowHeight();
        const step = Math.max(12, t.modes.mouseTrackingMode !== 'none' ? row * 3 : row);
        pending += rest;
        while (Math.abs(pending) >= step) {
            const dir = Math.sign(pending);
            wheel(dir, WheelEvent.DOM_DELTA_LINE);
            pending -= dir * step;
        }
    };

    const stopFling = () => {
        cancelAnimationFrame(fling);
        fling = 0;
    };

    const onStart = (e: TouchEvent) => {
        stopFling();
        tracking = e.touches.length === 1;
        if (!tracking) return;
        lastX = e.touches[0].clientX;
        lastY = e.touches[0].clientY;
        lastTime = e.timeStamp;
        axis = null;
        pending = 0;
        velocity = 0;
    };

    const onMove = (e: TouchEvent) => {
        if (!tracking || e.touches.length !== 1) {
            tracking = false;
            return;
        }
        const x = e.touches[0].clientX;
        const y = e.touches[0].clientY;
        if (!axis) {
            // A small wobble is still a tap.
            if (Math.abs(x - lastX) < 6 && Math.abs(y - lastY) < 6) return;
            axis = Math.abs(x - lastX) > Math.abs(y - lastY) ? 'x' : 'y';
        }
        // Ours, not xterm's (which would only move its scrollback) nor the page's.
        e.stopPropagation();
        if (e.cancelable) e.preventDefault();
        if (axis === 'x') {
            box.scrollLeft += lastX - x;
        } else {
            const dy = lastY - y;
            const dt = Math.max(1, e.timeStamp - lastTime);
            velocity = 0.8 * (dy / dt) + 0.2 * velocity;
            scrollY(dy);
        }
        lastX = x;
        lastY = y;
        lastTime = e.timeStamp;
    };

    const onEnd = (e: TouchEvent) => {
        if (!tracking) return;
        tracking = false;
        // A swipe that stopped before letting go doesn't fling.
        if (axis !== 'y' || e.timeStamp - lastTime > 80 || Math.abs(velocity) < 0.2) return;
        let v = velocity;
        let prev = performance.now();
        const step = (now: number) => {
            const dt = Math.min(32, now - prev);
            prev = now;
            scrollY(v * dt);
            v *= Math.pow(0.95, dt / 16);
            fling = Math.abs(v) > 0.03 ? requestAnimationFrame(step) : 0;
        };
        fling = requestAnimationFrame(step);
    };

    // Capture: before xterm's own handlers on the terminal inside the box.
    box.addEventListener('touchstart', onStart, { capture: true, passive: true });
    box.addEventListener('touchmove', onMove, { capture: true, passive: false });
    box.addEventListener('touchend', onEnd, { capture: true, passive: true });
    box.addEventListener('touchcancel', onEnd, { capture: true, passive: true });

    return () => {
        stopFling();
        box.removeEventListener('touchstart', onStart, { capture: true });
        box.removeEventListener('touchmove', onMove, { capture: true });
        box.removeEventListener('touchend', onEnd, { capture: true });
        box.removeEventListener('touchcancel', onEnd, { capture: true });
    };
}

function useCoarsePointer(): boolean {
    return useSyncExternalStore(
        (cb) => {
            const mql = window.matchMedia('(pointer: coarse)');
            mql.addEventListener('change', cb);
            return () => mql.removeEventListener('change', cb);
        },
        () => window.matchMedia('(pointer: coarse)').matches,
        () => false,
    );
}

/**
 * The session's real terminal, streamed from the machine through the server.
 *
 * Opening the view sends `watch`; the machine answers with a snapshot (sized to the pty, so
 * agents' full-screen output isn't garbled), then live output. Keystrokes go back as `input`:
 * click the terminal and type. Until the machine streams, the last preview lines are shown.
 *
 * The pty keeps the size Termlab gave it, so the font shrinks to fit its width (an appearance
 * option), and the view stays pinned to the bottom, where the prompt is, until you scroll up.
 */
export function LiveTerminal({
    sessionId,
    preview,
    interactive,
    canResize,
}: {
    sessionId: string;
    preview: string[];
    interactive: boolean;
    /** The machine's Termlab takes a viewer's size (0.2.9+); otherwise Termlab's size is kept. */
    canResize: boolean;
}) {
    const live = useLive();
    const chosen = useAppearance();
    const appearance = useMemo(
        () => (canResize || chosen.terminalSize === 'app' ? chosen : { ...chosen, terminalSize: 'app' as const }),
        [chosen, canResize],
    );
    const coarse = useCoarsePointer();
    const box = useRef<HTMLDivElement>(null);
    const term = useRef<Terminal | null>(null);
    const fitAddon = useRef<FitAddon | null>(null);
    const streaming = useRef(false);
    /** The session's pty size, once the machine says; until then the terminal fits the box. */
    const ptySize = useRef<{ cols: number; rows: number } | null>(null);
    /** Follow new output at the bottom; off while you've scrolled up to read. */
    const stick = useRef(true);
    const interactiveRef = useRef(interactive);
    const appearanceRef = useRef(appearance);
    const [isLive, setLive] = useState(false);

    interactiveRef.current = interactive;
    appearanceRef.current = appearance;

    /** The size this view last gave the session (null: Termlab's window has it). */
    const claimed = useRef<{ cols: number; rows: number } | null>(null);
    const claimTimer = useRef<number | undefined>(undefined);

    /**
     * Gives the session this view's size, in "Fit this browser" mode, while the page is
     * visible and the machine streams. `force` sends it again even if unchanged (Termlab may
     * have taken it back in the meantime).
     */
    const claim = (force = false) => {
        window.clearTimeout(claimTimer.current);
        claimTimer.current = window.setTimeout(() => {
            const t = term.current;
            if (!t || appearanceRef.current.terminalSize !== 'browser' || !streaming.current || !interactiveRef.current) return;
            if (document.visibilityState !== 'visible') return;
            const size = { cols: t.cols, rows: t.rows };
            if (!force && claimed.current?.cols === size.cols && claimed.current?.rows === size.rows) return;
            claimed.current = size;
            live.resize(sessionId, size);
        }, 150);
    };
    /** Hands the size back to Termlab's window. */
    const release = () => {
        window.clearTimeout(claimTimer.current);
        if (claimed.current) {
            claimed.current = null;
            live.resize(sessionId, null);
        }
    };

    const toBottom = () => {
        const b = box.current;
        if (b && stick.current) {
            b.scrollTop = b.scrollHeight;
        }
    };

    /** Applies the font (fitted to the box when the pty size is known) and the size. */
    const layout = () => {
        const t = term.current;
        const b = box.current;
        if (!t || !b) {
            return;
        }
        const a = appearanceRef.current;
        if (a.terminalSize === 'browser') {
            // This view's own size, at your font size; the session follows it.
            if (t.options.fontSize !== a.terminalFontSize) t.options.fontSize = a.terminalFontSize;
            if (t.options.lineHeight !== a.terminalLineHeight) t.options.lineHeight = a.terminalLineHeight;
            const dims = fitAddon.current?.proposeDimensions();
            if (dims && dims.cols >= 20 && dims.rows >= 5 && Number.isFinite(dims.cols)) {
                if (t.cols !== dims.cols || t.rows !== dims.rows) t.resize(dims.cols, dims.rows);
                claim();
            }
            requestAnimationFrame(toBottom);
            return;
        }
        let fontSize = a.terminalFontSize;
        const size = ptySize.current;
        if (size && a.fitWidth) {
            const narrow = window.matchMedia('(max-width: 767px)').matches;
            const fits = Math.floor((b.clientWidth / (size.cols * cellWidth(1))) * 10) / 10;
            fontSize = Math.max(Math.min(fontSize, fits), narrow ? MIN_FIT.narrow : MIN_FIT.wide);
        }
        if (t.options.fontSize !== fontSize) t.options.fontSize = fontSize;
        if (t.options.lineHeight !== a.terminalLineHeight) t.options.lineHeight = a.terminalLineHeight;
        if (size) {
            if (t.cols !== size.cols || t.rows !== size.rows) t.resize(size.cols, size.rows);
            if (a.fitWidth) {
                // xterm rounds cells to whole device pixels, so the estimate can be a little
                // wide: step down until the rendered screen really fits.
                const min = window.matchMedia('(max-width: 767px)').matches ? MIN_FIT.narrow : MIN_FIT.wide;
                const screen = b.querySelector<HTMLElement>('.xterm-screen');
                for (let i = 0; screen && i < 24 && screen.offsetWidth > b.clientWidth && fontSize > min; i++) {
                    fontSize = Math.max(min, Math.round((fontSize - 0.25) * 100) / 100);
                    t.options.fontSize = fontSize;
                }
            }
        } else {
            fitAddon.current?.fit();
        }
        requestAnimationFrame(toBottom);
    };

    // The terminal itself.
    useEffect(() => {
        const b = box.current;
        if (!b) {
            return;
        }
        const a = appearanceRef.current;
        const t = new Terminal({
            convertEol: true,
            cursorBlink: a.cursorBlink,
            fontFamily: FONT,
            fontSize: a.terminalFontSize,
            lineHeight: a.terminalLineHeight,
            scrollback: 5000,
            theme: theme(a),
        });
        const fit = new FitAddon();
        t.loadAddon(fit);
        t.open(b);
        term.current = t;
        fitAddon.current = fit;
        stick.current = true;
        layout();

        const onScroll = () => {
            stick.current = b.scrollTop + b.clientHeight >= b.scrollHeight - 8;
        };
        b.addEventListener('scroll', onScroll, { passive: true });
        const observer = new ResizeObserver(() => layout());
        observer.observe(b);
        // Webfonts can land after the first measure.
        void document.fonts?.ready.then(() => {
            cellRatio = 0;
            layout();
        });

        const stopSwipe = swipeToScroll(b, t);

        return () => {
            stopSwipe();
            observer.disconnect();
            b.removeEventListener('scroll', onScroll);
            t.dispose();
            term.current = null;
            fitAddon.current = null;
        };
        // layout reads refs only.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [sessionId]);

    // Appearance changes apply live.
    useEffect(() => {
        const t = term.current;
        if (!t) {
            return;
        }
        t.options.theme = theme(appearance);
        t.options.cursorBlink = appearance.cursorBlink;
        if (appearance.terminalSize === 'app') {
            release();
        }
        layout();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [appearance]);

    // Preview lines until real output arrives.
    useEffect(() => {
        const t = term.current;
        if (!t || streaming.current) {
            return;
        }

        t.reset();
        t.write(preview.join('\r\n'), toBottom);
    }, [preview, sessionId]);

    // The stream.
    useEffect(() => {
        streaming.current = false;
        ptySize.current = null;
        setLive(false);

        const stop = live.watch(sessionId, (event) => {
            const t = term.current;
            if (event.type === 'watch') {
                setLive(event.live);
                if (!event.live) {
                    streaming.current = false;
                }

                return;
            }
            if (!t) {
                return;
            }
            if (event.snapshot) {
                if (event.cols && event.rows) {
                    ptySize.current = { cols: event.cols, rows: event.rows };
                }
                t.reset();
                stick.current = true;
                if (appearanceRef.current.terminalSize === 'browser') {
                    // The snapshot was drawn at the session's current size: show it at that
                    // size, then take this view's size (the agent redraws for it).
                    if (event.cols && event.rows) t.resize(event.cols, event.rows);
                    streaming.current = true;
                    setLive(true);
                    t.write(decode(event.data), () => {
                        claimed.current = null;
                        layout();
                        claim(true);
                    });
                    return;
                }
                layout();
            } else if (!streaming.current) {
                t.reset();
            }
            streaming.current = true;
            setLive(true);
            t.write(decode(event.data), toBottom);
        });

        const keys = term.current?.onData((data) => {
            if (interactiveRef.current && streaming.current) {
                live.input(sessionId, encode(data));
            }
        });
        // Mouse reports in the old X10 format: one byte per character.
        const bytes = term.current?.onBinary((data) => {
            if (interactiveRef.current && streaming.current) {
                live.input(sessionId, btoa(data));
            }
        });

        // Coming back to this page takes the size again (Termlab may have taken it back);
        // leaving it hands the size back.
        const onVisible = () => (document.visibilityState === 'visible' ? claim(true) : release());
        const onFocus = () => claim(true);
        document.addEventListener('visibilitychange', onVisible);
        window.addEventListener('focus', onFocus);
        const b = box.current;
        b?.addEventListener('pointerdown', onFocus);

        return () => {
            keys?.dispose();
            bytes?.dispose();
            document.removeEventListener('visibilitychange', onVisible);
            window.removeEventListener('focus', onFocus);
            b?.removeEventListener('pointerdown', onFocus);
            release();
            stop();
        };
        // live.watch and live.input are stable; layout reads refs only.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [sessionId]);

    const send = (seq: string) => {
        if (interactive && streaming.current) {
            live.input(sessionId, encode(seq));
            stick.current = true;
        }
    };

    return (
        <div className="flex min-h-0 flex-1 flex-col">
            <div className="relative min-h-[200px] flex-1 overflow-hidden" style={{ background: theme(appearance).background }}>
                <div ref={box} className="term-box" aria-label="Session terminal" role="region" />
                {!isLive && (
                    <span className="pointer-events-none absolute top-2.5 right-3 rounded-full border border-outline-strong bg-term px-2 py-0.5 text-[11px] text-dim">
                        {interactive ? 'Waiting for the machine · last reported lines' : 'Machine offline · last reported lines'}
                    </span>
                )}
            </div>
            {coarse && interactive && (
                <div className="key-bar" role="toolbar" aria-label="Keys">
                    <button
                        type="button"
                        className="key"
                        aria-label="Keyboard"
                        onPointerDown={(e) => e.preventDefault()}
                        title="Type into the session"
                        onClick={() => term.current?.focus()}
                    >
                        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                            <rect x="2" y="6" width="20" height="12" rx="2" />
                            <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10" />
                        </svg>
                    </button>
                    {KEYS.map((k) => (
                        <button
                            key={k.label}
                            type="button"
                            className="key"
                            aria-label={k.title}
                            title={k.title}
                            disabled={!isLive}
                            // Keeps the keyboard open: the terminal keeps focus.
                            onPointerDown={(e) => e.preventDefault()}
                            onClick={() => send(k.seq)}
                        >
                            {k.label}
                        </button>
                    ))}
                </div>
            )}
        </div>
    );
}
