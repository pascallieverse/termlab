import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type FormEvent, type ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';
import { Button } from '@/components/el/button';
import { Dialog } from '@/components/el/dialog';
import { useToast } from '@/components/el/toast';
import { inputClass } from '@/layouts/AuthLayout';
import { Icon } from '@/layouts/nav';
import { useLive } from '@/live/LiveProvider';
import { ApiError, api, errorMessage } from '@/lib/api';
import { cn } from '@/lib/utils';
import { codeForChar, defaultSwap, isMacTarget, mapCode, modifiers, shortcuts } from './keys';
import { ScreenClient, webCodecsAvailable, type CursorShape, type Quality, type RemoteCursor, type ScreenHello, type ScreenState, type ScreenStats } from './screen-client';

type Phase = { kind: 'starting' } | { kind: 'code'; error: string | null } | { kind: 'session'; sid: string } | { kind: 'failed'; message: string };
type Fit = 'fit' | 'actual';
type TouchMode = 'trackpad' | 'touch';
type Pt = { x: number; y: number };
type Size = { w: number; h: number };
type ZoomPan = { zoom: number; pan: Pt };

const KEYBAR: { label: string; code: string; title: string }[] = [
    { label: 'Esc', code: 'Escape', title: 'Escape' },
    { label: 'Tab', code: 'Tab', title: 'Tab' },
    { label: '←', code: 'ArrowLeft', title: 'Left' },
    { label: '↑', code: 'ArrowUp', title: 'Up' },
    { label: '↓', code: 'ArrowDown', title: 'Down' },
    { label: '→', code: 'ArrowRight', title: 'Right' },
    { label: '⌫', code: 'Backspace', title: 'Delete' },
    { label: '⏎', code: 'Enter', title: 'Return' },
    { label: '⌦', code: 'Delete', title: 'Forward delete' },
    { label: 'Home', code: 'Home', title: 'Home' },
    { label: 'End', code: 'End', title: 'End' },
    { label: 'PgUp', code: 'PageUp', title: 'Page up' },
    { label: 'PgDn', code: 'PageDown', title: 'Page down' },
    ...Array.from({ length: 12 }, (_, i) => ({ label: `F${i + 1}`, code: `F${i + 1}`, title: `F${i + 1}` })),
];

const QUALITIES: { value: Quality; label: string; short: string }[] = [
    { value: 'low', label: 'Low (data saver)', short: 'Low' },
    { value: 'balanced', label: 'Balanced', short: 'Balanced' },
    { value: 'high', label: 'Sharp (4K)', short: 'Sharp' },
    { value: 'native', label: 'Native', short: 'Native' },
];

const GESTURES: Record<TouchMode, [string, string][]> = {
    trackpad: [
        ['Move a finger', 'move the pointer'],
        ['Tap', 'click (tap twice: double-click)'],
        ['Tap with two fingers', 'right-click'],
        ['Hold, then move', 'drag'],
        ['Two fingers', 'scroll'],
        ['Pinch', 'zoom in on the picture'],
    ],
    touch: [
        ['Tap', 'click there (tap twice: double-click)'],
        ['Hold and let go', 'right-click'],
        ['Hold, then move', 'drag'],
        ['Swipe', 'scroll'],
        ['Pinch', 'zoom in on the picture'],
        ['Two fingers', 'move around while zoomed in'],
    ],
};

/** Keep this much text in the hidden phone-keyboard field, so Backspace has something to delete. */
const KB_PAD = '  ';

/** Browsers don't show cursor images larger than this (CSS pixels). */
const MAX_CSS_CURSOR = 128;

/** Touches: how long a press is a hold, how far a finger goes before it's a move, how far one can zoom. */
const HOLD_MS = 450;
const SLOP = 8;
const MAX_ZOOM = 6;

/** Phones, and windows too small for every control in the bar: the rest go in a sheet. */
const COMPACT = '(max-width: 959px), (max-height: 520px)';

/**
 * The computer's pointer picture as a CSS cursor, `w`×`h` CSS pixels: candidates from best
 * (sharp on high-density screens) to plainest, for browsers that reject the first.
 */
function cssCursor(shape: CursorShape, w: number, h: number): string[] {
    const dpr = Math.min(3, Math.max(1, window.devicePixelRatio || 1));
    const png = (f: number) => {
        const c = document.createElement('canvas');
        c.width = Math.max(1, Math.round(w * f));
        c.height = Math.max(1, Math.round(h * f));
        const ctx = c.getContext('2d');
        if (ctx) {
            ctx.imageSmoothingQuality = 'high';
            ctx.drawImage(shape.image, 0, 0, c.width, c.height);
        }

        return c.toDataURL('image/png');
    };
    const hx = Math.min(w - 1, Math.round((shape.hotX * w) / shape.width));
    const hy = Math.min(h - 1, Math.round((shape.hotY * h) / shape.height));
    const one = png(1);
    const plain = `url(${one}) ${hx} ${hy}, default`;
    if (dpr === 1) return [plain];
    const set = `url(${one}) 1x, url(${png(dpr)}) ${dpr}x`;

    return [`image-set(${set}) ${hx} ${hy}, default`, `-webkit-image-set(${set}) ${hx} ${hy}, default`, plain];
}

/**
 * Where a `video`-sized picture goes in `area`: fitted, then zoomed and moved by `pan` (from the
 * centre). The pan comes back limited, so the picture never leaves a gap it could fill.
 */
function place(video: Size, area: Size, zoom: number, pan: Pt) {
    const s = Math.min(area.w / video.w, area.h / video.h) * zoom;
    const w = video.w * s;
    const h = video.h * s;
    const limit = (size: number, room: number, p: number) => (size <= room ? 0 : Math.max(-(size - room) / 2, Math.min((size - room) / 2, p)));
    const x = limit(w, area.w, pan.x);
    const y = limit(h, area.h, pan.y);

    return { w, h, left: (area.w - w) / 2 + x, top: (area.h - h) / 2 + y, pan: { x, y } };
}

const clamp01 = (p: Pt): Pt => ({ x: Math.min(1, Math.max(0, p.x)), y: Math.min(1, Math.max(0, p.y)) });
const onPicture = (p: Pt) => p.x >= 0 && p.x <= 1 && p.y >= 0 && p.y <= 1;
/** Controls take their own touches (the picture's gestures leave them alone). */
const isControl = (t: EventTarget | null) => t instanceof Element && t.closest('button, a, input, select, textarea, [data-control]') !== null;

/** A media query's answer, kept current. */
function useMedia(query: string): boolean {
    const subscribe = useCallback(
        (changed: () => void) => {
            const m = window.matchMedia(query);
            m.addEventListener('change', changed);

            return () => m.removeEventListener('change', changed);
        },
        [query],
    );

    return useSyncExternalStore(
        subscribe,
        () => window.matchMedia(query).matches,
        () => false,
    );
}

/**
 * The part of the page in view while a phone's keyboard is up (null when it isn't). The viewer
 * shrinks to it, so the key bar sits on the keyboard instead of behind it.
 */
function useKeyboardViewport(): { top: number; height: number } | null {
    const [vp, setVp] = useState<{ top: number; height: number } | null>(null);
    useEffect(() => {
        const vv = window.visualViewport;
        if (!vv) return;
        const on = () => {
            // Pinch-zooming the page shrinks the visual viewport too; that's not a keyboard.
            const up = vv.scale < 1.05 && document.documentElement.clientHeight - vv.height > 120;
            setVp((old) => {
                if (!up) return null;
                if (old && old.top === vv.offsetTop && old.height === vv.height) return old;

                return { top: vv.offsetTop, height: vv.height };
            });
        };
        on();
        vv.addEventListener('resize', on);
        vv.addEventListener('scroll', on);

        return () => {
            vv.removeEventListener('resize', on);
            vv.removeEventListener('scroll', on);
        };
    }, []);

    return vp;
}

/**
 * Seeing and controlling a computer's screen. Opens over everything (the sessions keep
 * streaming behind it). Asks for an authenticator code first, unless one was given recently.
 */
export function ScreenViewer({ machineId }: { machineId: number }) {
    const live = useLive();
    const navigate = useNavigate();
    const toast = useToast();
    const machine = live.machines.find((m) => m.id === machineId);
    const [phase, setPhase] = useState<Phase>({ kind: 'starting' });
    const [code, setCode] = useState('');
    const [busy, setBusy] = useState(false);
    const keyboard = useKeyboardViewport();
    const close = useCallback(() => navigate('/remote'), [navigate]);

    const begin = useCallback(
        async (totp?: string) => {
            if (!webCodecsAvailable()) {
                setPhase({ kind: 'failed', message: 'This browser can’t show a remote screen (it has no WebCodecs). Use a recent Chrome, Edge, Firefox or Safari.' });
                return;
            }
            setBusy(true);
            try {
                const r = await api.post<{ sid: string }>(`/api/machines/${machineId}/screen`, totp ? { totp_code: totp } : {});
                setPhase({ kind: 'session', sid: r.sid });
            } catch (e) {
                if (e instanceof ApiError && e.status === 428) {
                    setPhase({ kind: 'code', error: null });
                } else if (e instanceof ApiError && (e.status === 422 || e.status === 429)) {
                    setPhase({ kind: 'code', error: e.message });
                } else {
                    setPhase({ kind: 'failed', message: errorMessage(e, 'The screen session couldn’t start.') });
                }
            } finally {
                setBusy(false);
                setCode('');
            }
        },
        [machineId],
    );

    useEffect(() => {
        void begin();
    }, [begin]);

    const submit = (e: FormEvent) => {
        e.preventDefault();
        if (code.trim().length >= 6) {
            void begin(code.trim());
        }
    };

    const name = machine?.name ?? 'Computer';

    return (
        <div
            className="screen-viewer fixed inset-0 z-[60] flex flex-col bg-black pt-[env(safe-area-inset-top)] pr-[env(safe-area-inset-right)] pl-[env(safe-area-inset-left)] text-sm text-ink"
            style={keyboard ? { top: keyboard.top, height: keyboard.height, bottom: 'auto' } : undefined}
            role="dialog"
            aria-modal="true"
            aria-label={`Screen of ${name}`}
        >
            {phase.kind === 'session' ? (
                <Session key={phase.sid} sid={phase.sid} name={name} onClose={close} onRestart={() => void begin()} toast={toast} keyboardUp={keyboard !== null} />
            ) : (
                <>
                    <Bar name={name} onClose={close} />
                    <div className="grid flex-1 place-items-center p-4">
                        {phase.kind === 'starting' && <Card>Starting…</Card>}
                        {phase.kind === 'failed' && (
                            <Card>
                                <p className="m-0 mb-4 text-ink-soft">{phase.message}</p>
                                <div className="flex justify-center gap-2">
                                    <Button variant="outline" onClick={close}>
                                        Close
                                    </Button>
                                    <Button onClick={() => void begin()}>Try again</Button>
                                </div>
                            </Card>
                        )}
                        {phase.kind === 'code' && (
                            <Card>
                                <form onSubmit={submit} className="flex flex-col gap-3 text-left">
                                    <h2 className="m-0 text-base font-semibold">Control {name}</h2>
                                    <p className="m-0 text-ink-soft">You’ll see its screen and use its mouse and keyboard. Enter a code from your authenticator app to continue.</p>
                                    <label className="flex flex-col gap-1.5">
                                        <span className="text-[13px] text-dim">Authenticator code</span>
                                        <input
                                            autoFocus
                                            inputMode="numeric"
                                            autoComplete="one-time-code"
                                            pattern="[0-9 ]*"
                                            maxLength={8}
                                            value={code}
                                            onChange={(e) => setCode(e.target.value.replace(/[^0-9]/g, ''))}
                                            className={`${inputClass} text-center font-mono text-lg tracking-[0.3em]`}
                                            aria-invalid={phase.error ? true : undefined}
                                            aria-describedby={phase.error ? 'screen-code-error' : undefined}
                                        />
                                    </label>
                                    {phase.error && (
                                        <p id="screen-code-error" className="m-0 text-[13px] text-bad">
                                            {phase.error}
                                        </p>
                                    )}
                                    <div className="mt-1 flex justify-end gap-2">
                                        <Button variant="outline" onClick={close}>
                                            Cancel
                                        </Button>
                                        <Button type="submit" disabled={busy || code.length < 6}>
                                            {busy ? 'Checking…' : 'Continue'}
                                        </Button>
                                    </div>
                                </form>
                            </Card>
                        )}
                    </div>
                </>
            )}
        </div>
    );
}

function Card({ children }: { children: ReactNode }) {
    return <div className="w-full max-w-[380px] rounded-xl border border-line bg-panel p-5 text-center shadow-2xl">{children}</div>;
}

function Bar({ name, onClose, children }: { name: string; onClose: () => void; children?: ReactNode }) {
    return (
        <header className="screen-bar flex shrink-0 items-center gap-2 overflow-x-auto border-b border-line bg-surface px-2 py-1.5 [scrollbar-width:none]">
            <button type="button" className="icon-btn" onClick={onClose} aria-label="End and close" title="End and close">
                <Icon name="close" size={16} />
            </button>
            <span className="flex min-w-0 shrink items-center gap-1.5 font-medium">
                <Icon name="screen" size={16} />
                <span className="max-w-[160px] truncate">{name}</span>
            </span>
            {children}
        </header>
    );
}

/** Small line icons for the viewer's own buttons. */
function Glyph({ name }: { name: 'keyboard' | 'more' | 'expand' | 'shrink' | 'bars' }) {
    const d = {
        keyboard: 'M3 6h18v12H3zM7 10h.01M11 10h.01M15 10h.01M7 14h10',
        more: 'M5 12h.01M12 12h.01M19 12h.01',
        expand: 'M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5',
        shrink: 'M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5',
        bars: 'M4 7h16M4 12h16M4 17h16',
    }[name];

    return (
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={name === 'more' ? 3 : 2} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d={d} />
        </svg>
    );
}

type Toast = ReturnType<typeof useToast>;

type Touch = { x: number; y: number; sx: number; sy: number; t: number };
type Gesture = {
    /** The first finger: it alone moves the pointer and drags. */
    primary: number;
    /** The most fingers down at once. */
    fingers: number;
    moved: boolean;
    /** Pressed without moving long enough. */
    held: boolean;
    dragging: boolean;
    hold: number;
    started: number;
    /** Touch mode: the first finger landed on the picture (not the black around it). */
    onPicture: boolean;
    /** Two fingers: zoom and move the picture here, or scroll the computer. */
    two: 'undecided' | 'view' | 'scroll';
    dist0: number;
    mid0: Pt;
    /** Where the fingers were when the picture last moved (it moves once a frame). */
    last: { m: Pt; d: number } | null;
    /** Recent scrolling (computer pixels), for a fling when the fingers let go. */
    samples: { t: number; dx: number; dy: number }[];
};

function Session({ sid, name, onClose, onRestart, toast, keyboardUp }: { sid: string; name: string; onClose: () => void; onRestart: () => void; toast: Toast; keyboardUp: boolean }) {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const areaRef = useRef<HTMLDivElement>(null);
    const rootRef = useRef<HTMLDivElement>(null);
    const kbRef = useRef<HTMLInputElement>(null);
    const clientRef = useRef<ScreenClient | null>(null);
    const [state, setState] = useState<ScreenState>({ state: 'connecting' });
    const [hello, setHello] = useState<ScreenHello | null>(null);
    const [display, setDisplay] = useState<number | null>(null);
    const [quality, setQuality] = useState<Quality>('balanced');
    const [video, setVideo] = useState<Size | null>(null);
    const [area, setArea] = useState<Size>({ w: 0, h: 0 });
    const [stats, setStats] = useState<ScreenStats | null>(null);
    const [fit, setFit] = useState<Fit>('fit');
    const [view, setViewState] = useState<ZoomPan>({ zoom: 1, pan: { x: 0, y: 0 } });
    const touchDevice = useMemo(() => matchMedia('(pointer: coarse)').matches, []);
    const compact = useMedia(COMPACT);
    const [touchMode, setTouchMode] = useState<TouchMode>('trackpad');
    const [swap, setSwap] = useState(false);
    const [latched, setLatched] = useState<string[]>([]);
    const [keyboardOpen, setKeyboardOpen] = useState(false);
    const [menu, setMenu] = useState<'keys' | 'clipboard' | null>(null);
    const [sheet, setSheet] = useState(false);
    /** The bars are shown (on a phone they can be put away for a bigger picture). */
    const [chrome, setChrome] = useState(true);
    const [clipText, setClipText] = useState<{ mode: 'from' | 'to'; text: string } | null>(null);
    const [fullscreen, setFullscreen] = useState(false);
    const canFullscreen = useMemo(() => document.fullscreenEnabled === true, []);
    const canControl = hello?.input ?? false;
    const live = state.state === 'live';
    const zoomed = view.zoom > 1.01;
    const overlayRef = useRef<HTMLCanvasElement>(null);
    /** The computer's pointer, as last heard (positions from here while a finger moves it). */
    const remote = useRef<RemoteCursor | null>(null);
    /** A mouse is over the picture: the browser's own pointer shows the computer's shape. */
    const hovering = useRef(false);
    const localUntil = useRef(0);
    const cursorCache = useRef(new Map<string, string[]>());
    const paint = useRef<{ layout: { w: number; h: number; left: number; top: number } | null; pointsWide: number; control: boolean; live: boolean }>({
        layout: null,
        pointsWide: 0,
        control: false,
        live: false,
    });
    /** Sizes and zoom as the gesture handlers need them now (between renders too). */
    const geo = useRef<{ video: Size | null; area: Size }>({ video: null, area: { w: 0, h: 0 } });
    const viewRef = useRef<ZoomPan>(view);

    // The session itself.
    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const client = new ScreenClient(sid, canvas, {
            onState: setState,
            onHello: (h) => {
                setHello(h);
                setSwap(defaultSwap(h.os));
                setDisplay(h.display);
                setQuality(h.quality);
            },
            onDisplay: setDisplay,
            onQuality: setQuality,
            onSize: (w, h) => setVideo({ w, h }),
            onClipboard: (text) => {
                // Copying needs a click in most browsers; show it with a Copy button.
                setClipText({ mode: 'from', text });
            },
            onClipboardSaved: (ok) => (ok ? toast.success('Copied to the computer’s clipboard.') : toast.error('The computer’s clipboard couldn’t be set.')),
            onStats: setStats,
            onCursor: (c) => {
                // A finger just moved it: keep showing where the finger put it.
                const prev = remote.current;
                remote.current = performance.now() < localUntil.current && prev ? { ...c, x: prev.x, y: prev.y } : c;
                paintCursor();
            },
        });
        clientRef.current = client;

        return () => {
            client.close();
            clientRef.current = null;
        };
    }, [sid, toast]);

    // The viewing area's size.
    useLayoutEffect(() => {
        const el = areaRef.current;
        if (!el) return;
        const ro = new ResizeObserver(() => setArea({ w: el.clientWidth, h: el.clientHeight }));
        ro.observe(el);
        setArea({ w: el.clientWidth, h: el.clientHeight });

        return () => ro.disconnect();
    }, []);

    useEffect(() => {
        const on = () => setFullscreen(!!document.fullscreenElement);
        document.addEventListener('fullscreenchange', on);

        return () => document.removeEventListener('fullscreenchange', on);
    }, []);

    // Where the picture goes: fitted (and zoomed, on a phone), or one pixel per pixel.
    const layout = useMemo(() => {
        if (!video || area.w === 0) return null;
        if (fit === 'actual') {
            return { w: video.w, h: video.h, left: 0, top: 0 };
        }

        return place(video, area, view.zoom, view.pan);
    }, [video, area, fit, view]);

    useLayoutEffect(() => {
        geo.current = { video, area };
    }, [video, area]);

    /** Zooms (1–6×) and moves the picture, kept where it fills what it can. */
    const setView = useCallback((zoom: number, pan: Pt) => {
        const { video: v, area: a } = geo.current;
        if (!v || !a.w) return;
        const z = Math.min(MAX_ZOOM, Math.max(1, zoom));
        const next = { zoom: z, pan: place(v, a, z, pan).pan };
        viewRef.current = next;
        setViewState(next);
    }, []);

    const resetZoom = () => setView(1, { x: 0, y: 0 });

    /** Moves a zoomed-in picture so this point (0–1 on the picture) stays in sight. */
    const follow = useCallback(
        (nx: number, ny: number) => {
            const { video: v, area: a } = geo.current;
            const { zoom, pan } = viewRef.current;
            if (!v || !a.w || zoom <= 1) return;
            const l = place(v, a, zoom, pan);
            const px = l.left + nx * l.w;
            const py = l.top + ny * l.h;
            const m = Math.min(48, a.w / 4, a.h / 4);
            let dx = 0;
            let dy = 0;
            if (px < m) dx = m - px;
            else if (px > a.w - m) dx = a.w - m - px;
            if (py < m) dy = m - py;
            else if (py > a.h - m) dy = a.h - m - py;
            if (dx || dy) setView(zoom, { x: pan.x + dx, y: pan.y + dy });
        },
        [setView],
    );

    // The area changed size (the phone's keyboard came up, the phone turned): keep a zoomed-in
    // picture in bounds, with the pointer in sight.
    useEffect(() => {
        const { zoom, pan } = viewRef.current;
        if (zoom <= 1) return;
        setView(zoom, pan);
        const c = remote.current;
        if (c && onPicture(c)) follow(c.x, c.y);
    }, [area, video, setView, follow]);

    // ---- the computer's pointer -------------------------------------------------------------

    /**
     * Shows the computer's pointer: as the browser's own pointer while a mouse is over the
     * picture (it moves at once, the shape follows the computer), otherwise drawn over the
     * picture where the computer has it.
     */
    const paintCursor = useCallback(() => {
        const el = canvasRef.current;
        const ov = overlayRef.current;
        if (!el || !ov) return;
        const { layout: l, pointsWide, control, live: on } = paint.current;
        const c = remote.current;
        const local = hovering.current && control;
        const fallback = control ? 'default' : 'not-allowed';
        const shape = c?.shape ?? null;
        // CSS pixels per display point, and the picture's size on screen.
        const k = l && pointsWide ? l.w / pointsWide : 0;
        let w = shape ? (shape.width / shape.scale) * k : 0;
        let h = shape ? (shape.height / shape.scale) * k : 0;
        const fit = Math.min(1, MAX_CSS_CURSOR / Math.max(w, h, 1));
        w = Math.max(1, Math.round(w * fit));
        h = Math.max(1, Math.round(h * fit));

        if (!local || !c) {
            el.style.cursor = fallback;
        } else if (!c.visible) {
            el.style.cursor = 'none';
        } else if (shape && k) {
            const key = `${shape.id}:${w}x${h}`;
            let css = cursorCache.current.get(key);
            if (!css) {
                if (cursorCache.current.size > 64) cursorCache.current.clear();
                css = cssCursor(shape, w, h);
                cursorCache.current.set(key, css);
            }
            for (const candidate of css) {
                el.style.cursor = candidate;
                if (el.style.cursor && el.style.cursor !== fallback) break;
            }
        } else {
            el.style.cursor = fallback;
        }

        const show = on && !local && !!c && c.visible && !!shape && !!l && k > 0 && c.x >= 0 && c.x <= 1 && c.y >= 0 && c.y <= 1;
        if (!show || !shape || !l || !c) {
            ov.style.display = 'none';
            return;
        }
        if (ov.dataset.id !== String(shape.id)) {
            ov.dataset.id = String(shape.id);
            ov.width = shape.width;
            ov.height = shape.height;
            const ctx = ov.getContext('2d');
            ctx?.clearRect(0, 0, ov.width, ov.height);
            ctx?.drawImage(shape.image, 0, 0);
        }
        // On a phone the picture is small: keep the pointer big enough to see.
        const grow = touchDevice ? Math.max(1, Math.min(2, 16 / Math.max(w, h, 1))) : 1;
        const sx = (w * grow) / shape.width;
        const sy = (h * grow) / shape.height;
        ov.style.display = 'block';
        ov.style.width = `${w * grow}px`;
        ov.style.height = `${h * grow}px`;
        ov.style.transform = `translate(${l.left + c.x * l.w - shape.hotX * sx}px, ${l.top + c.y * l.h - shape.hotY * sy}px)`;
    }, [touchDevice]);

    const pointsWide = hello?.displays.find((d) => d.id === display)?.width ?? 0;
    useLayoutEffect(() => {
        paint.current = { layout, pointsWide, control: canControl, live };
        paintCursor();
    }, [layout, pointsWide, canControl, live, paintCursor]);

    /** Moved here (by a finger): show it there before the computer says so. */
    const movedLocally = (x: number, y: number) => {
        localUntil.current = performance.now() + 400;
        if (remote.current) {
            remote.current = { ...remote.current, x, y };
            paintCursor();
        }
    };

    const onPointerEnter = (e: React.PointerEvent<HTMLCanvasElement>) => {
        if (e.pointerType === 'mouse' || e.pointerType === 'pen') {
            hovering.current = true;
            paintCursor();
        }
    };

    const onPointerLeave = (e: React.PointerEvent<HTMLCanvasElement>) => {
        if (hovering.current && (e.pointerType === 'mouse' || e.pointerType === 'pen')) {
            hovering.current = false;
            paintCursor();
        }
    };

    // ---- sending -------------------------------------------------------------------------

    /** A point on the screen as 0–1 across the picture (outside it: below 0 or above 1). */
    const norm = (clientX: number, clientY: number): Pt => {
        const r = canvasRef.current!.getBoundingClientRect();

        return { x: (clientX - r.left) / r.width, y: (clientY - r.top) / r.height };
    };

    /** A point on the screen in the viewing area's own pixels. */
    const inArea = (p: Pt): Pt => {
        const r = areaRef.current!.getBoundingClientRect();

        return { x: p.x - r.left, y: p.y - r.top };
    };

    // Moves and scrolls go out at most once a frame.
    const pendingMove = useRef<Pt | null>(null);
    const pendingWheel = useRef({ dx: 0, dy: 0 });
    const frame = useRef(0);
    /** Sends what's waiting now (before a press or release, so they keep their order). */
    const flushNow = () => {
        cancelAnimationFrame(frame.current);
        frame.current = 0;
        const c = clientRef.current;
        const p = pendingMove.current;
        pendingMove.current = null;
        if (p) c?.pointer(p.x, p.y, 'move');
        const w = pendingWheel.current;
        if (w.dx || w.dy) c?.wheel(Math.round(w.dx), Math.round(w.dy));
        pendingWheel.current = { dx: 0, dy: 0 };
    };
    const flush = () => {
        if (!frame.current) frame.current = requestAnimationFrame(flushNow);
    };
    /** A press or release, after any move still waiting. */
    const press = (x: number, y: number, action: 'down' | 'up', which = 0, clicks = 1) => {
        flushNow();
        clientRef.current?.pointer(x, y, action, which, clicks);
    };
    const sendMove = (x: number, y: number) => {
        pendingMove.current = { x, y };
        flush();
    };
    const sendWheel = (dx: number, dy: number) => {
        pendingWheel.current.dx += dx;
        pendingWheel.current.dy += dy;
        flush();
    };

    const click = (x: number, y: number, button: number, clicks = 1) => {
        const c = clientRef.current;
        if (!c) return;
        flushNow();
        c.pointer(x, y, 'move');
        c.pointer(x, y, 'down', button, clicks);
        c.pointer(x, y, 'up', button, clicks);
    };

    // ---- scrolling with fingers, and its fling ----------------------------------------------

    const fling = useRef(0);
    const stopFling = () => {
        cancelAnimationFrame(fling.current);
        fling.current = 0;
    };
    useEffect(() => () => cancelAnimationFrame(fling.current), []);

    /** Scrolls as the fingers moved (CSS pixels): the computer's content follows them. */
    const fingerScroll = (g: Gesture, dx: number, dy: number) => {
        const { layout: l, pointsWide: pw } = paint.current;
        const k = l && pw ? Math.max(1, pw / l.w) : 2;
        const wx = -dx * k;
        const wy = -dy * k;
        sendWheel(wx, wy);
        const now = performance.now();
        g.samples.push({ t: now, dx: wx, dy: wy });
        while (g.samples.length > 1 && now - g.samples[0].t > 100) g.samples.shift();
    };

    /** Fingers let go while scrolling: keep going for a moment, slowing down. */
    const flingFrom = (g: Gesture) => {
        const s = g.samples;
        const now = performance.now();
        if (s.length < 2 || now - s[s.length - 1].t > 60) return;
        const span = Math.max(16, now - s[0].t);
        let vx = s.reduce((a, b) => a + b.dx, 0) / span;
        let vy = s.reduce((a, b) => a + b.dy, 0) / span;
        if (Math.hypot(vx, vy) < 0.4) return;
        stopFling();
        let last = now;
        const step = (t: number) => {
            const dt = Math.min(48, Math.max(0, t - last));
            last = t;
            const k = Math.pow(0.95, dt / 16);
            vx *= k;
            vy *= k;
            if (Math.hypot(vx, vy) < 0.05) {
                fling.current = 0;
                return;
            }
            sendWheel(vx * dt, vy * dt);
            fling.current = requestAnimationFrame(step);
        };
        fling.current = requestAnimationFrame(step);
    };

    // ---- zooming and moving the picture with two fingers -------------------------------------

    /** The fingers went from (`m0`, `d0` apart) to (`m1`, `d1` apart), in area pixels. */
    const moveView = (m0: Pt, d0: number, m1: Pt, d1: number) => {
        const { video: v, area: a } = geo.current;
        if (!v || !a.w) return;
        const { zoom, pan } = viewRef.current;
        const l = place(v, a, zoom, pan);
        // The picture's point under the fingers stays under them.
        const u = (m0.x - l.left) / l.w;
        const w0 = (m0.y - l.top) / l.h;
        const z = Math.min(MAX_ZOOM, Math.max(1, zoom * (d0 > 0 ? d1 / d0 : 1)));
        const s = Math.min(a.w / v.w, a.h / v.h) * z;
        const w = v.w * s;
        const h = v.h * s;
        setView(z, { x: m1.x - u * w - (a.w - w) / 2, y: m1.y - w0 * h - (a.h - h) / 2 });
    };

    // ---- touch: trackpad (relative) or direct ------------------------------------------------

    const cursor = useRef<Pt>({ x: 0.5, y: 0.5 });
    const touches = useRef(new Map<number, Touch>());
    const gesture = useRef<Gesture | null>(null);
    const viewFrame = useRef(0);
    const lastTap = useRef<{ t: number; cx: number; cy: number; at: Pt } | null>(null);

    /** The two first fingers: their midpoint (client pixels) and distance. */
    const pair = () => {
        const [a, b] = [...touches.current.values()];

        return { m: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }, d: Math.hypot(a.x - b.x, a.y - b.y) };
    };

    const onTouchDown = (e: React.PointerEvent<HTMLDivElement>, c: ScreenClient) => {
        e.preventDefault();
        stopFling();
        e.currentTarget.setPointerCapture(e.pointerId);
        const now = performance.now();
        touches.current.set(e.pointerId, { x: e.clientX, y: e.clientY, sx: e.clientX, sy: e.clientY, t: now });
        const n = touches.current.size;
        const g0 = gesture.current;
        if (n === 1 || !g0) {
            const p = norm(e.clientX, e.clientY);
            const g: Gesture = {
                primary: e.pointerId,
                fingers: 1,
                moved: false,
                held: false,
                dragging: false,
                hold: 0,
                started: now,
                onPicture: onPicture(p),
                two: 'undecided',
                dist0: 0,
                mid0: { x: 0, y: 0 },
                last: null,
                samples: [],
            };
            if (touchMode === 'touch' && g.onPicture) {
                cursor.current = p;
                c.pointer(p.x, p.y, 'move');
                movedLocally(p.x, p.y);
            }
            if (touchMode === 'trackpad' || g.onPicture) {
                g.hold = window.setTimeout(() => {
                    if (g.moved || g.fingers > 1 || gesture.current !== g) return;
                    g.held = true;
                    navigator.vibrate?.(10);
                    // Trackpad: press now, so moving drags. Touch: wait to see if it moves (drag)
                    // or lets go (right-click).
                    if (touchMode === 'trackpad') {
                        g.dragging = true;
                        press(cursor.current.x, cursor.current.y, 'down');
                    }
                }, HOLD_MS);
            }
            gesture.current = g;
            return;
        }
        g0.fingers = Math.max(g0.fingers, n);
        // A second finger while dragging: the drag goes on, that finger is ignored.
        if (g0.dragging || n !== 2) return;
        window.clearTimeout(g0.hold);
        const { m, d } = pair();
        g0.two = 'undecided';
        g0.dist0 = d;
        g0.mid0 = m;
    };

    const onTouchMove = (e: React.PointerEvent<HTMLDivElement>) => {
        const t = touches.current.get(e.pointerId);
        const g = gesture.current;
        if (!t || !g) return;
        const two = touches.current.size >= 2 && !g.dragging;
        const before = two ? pair() : null;
        const now = performance.now();
        const dx = e.clientX - t.x;
        const dy = e.clientY - t.y;
        const dt = Math.max(1, now - t.t);
        t.x = e.clientX;
        t.y = e.clientY;
        t.t = now;
        if (!g.moved && Math.hypot(e.clientX - t.sx, e.clientY - t.sy) > SLOP) {
            g.moved = true;
            if (!g.held) window.clearTimeout(g.hold);
        }

        if (before) {
            const after = pair();
            if (g.two === 'undecided') {
                if (Math.abs(after.d - g.dist0) > 24) g.two = 'view';
                else if (Math.hypot(after.m.x - g.mid0.x, after.m.y - g.mid0.y) > 10) {
                    // Touch mode: two fingers move a zoomed-in picture (one finger scrolls).
                    g.two = touchMode === 'touch' && viewRef.current.zoom > 1 ? 'view' : 'scroll';
                } else return;
            } else if (g.two === 'scroll' && Math.abs(after.d - g.dist0) > 48) {
                g.two = 'view';
                g.samples = [];
            }
            if (g.two === 'scroll') {
                fingerScroll(g, after.m.x - before.m.x, after.m.y - before.m.y);
                return;
            }
            // Each finger reports on its own: move the picture once both have, each frame
            // (half-moved fingers look like a pinch, and the edges would eat the move).
            if (!viewFrame.current) {
                viewFrame.current = requestAnimationFrame(() => {
                    viewFrame.current = 0;
                    if (gesture.current !== g || touches.current.size < 2) return;
                    const now = pair();
                    if (g.last) moveView(inArea(g.last.m), g.last.d, inArea(now.m), now.d);
                    g.last = now;
                });
            }
            return;
        }

        // One finger: the first, unless a second one took part (then it waits for all to lift).
        if (e.pointerId !== g.primary || (g.fingers > 1 && !g.dragging)) return;
        if (touchMode === 'trackpad') {
            if (!g.moved && !g.dragging) return;
            // Faster moves go further: slow ones stay precise on a small screen.
            const speed = Math.hypot(dx, dy) / dt;
            const gain = Math.min(4, 0.9 + 1.3 * speed);
            const r = canvasRef.current!.getBoundingClientRect();
            cursor.current = clamp01({ x: cursor.current.x + (dx * gain) / r.width, y: cursor.current.y + (dy * gain) / r.height });
            sendMove(cursor.current.x, cursor.current.y);
            movedLocally(cursor.current.x, cursor.current.y);
            follow(cursor.current.x, cursor.current.y);
        } else if (g.held) {
            if (!g.dragging) {
                g.dragging = true;
                press(cursor.current.x, cursor.current.y, 'down');
            }
            const p = clamp01(norm(e.clientX, e.clientY));
            cursor.current = p;
            sendMove(p.x, p.y);
            movedLocally(p.x, p.y);
        } else if (g.moved) {
            fingerScroll(g, dx, dy);
        }
    };

    const onTouchUp = (e: React.PointerEvent<HTMLDivElement>, cancelled: boolean) => {
        if (!touches.current.delete(e.pointerId)) return;
        const g = gesture.current;
        if (!g) return;
        if (touches.current.size > 0) {
            // The dragging finger lifted: let go, even if another is still down.
            if (g.dragging && e.pointerId === g.primary) {
                g.dragging = false;
                g.fingers = Math.max(g.fingers, 2);
                press(cursor.current.x, cursor.current.y, 'up');
            }
            return;
        }
        window.clearTimeout(g.hold);
        gesture.current = null;
        const tapped = !cancelled && !g.dragging && !g.moved && !g.held && g.fingers === 1 && (touchMode === 'trackpad' || g.onPicture);
        if (!tapped) lastTap.current = null;
        if (g.dragging) {
            press(cursor.current.x, cursor.current.y, 'up');
            return;
        }
        if (cancelled) return;
        const now = performance.now();
        if (g.fingers > 1) {
            if (g.two === 'scroll') flingFrom(g);
            else if (g.fingers === 2 && g.two === 'undecided' && now - g.started < 500) click(cursor.current.x, cursor.current.y, 2);
            return;
        }
        if (touchMode === 'touch') {
            if (g.moved) {
                flingFrom(g);
                return;
            }
            if (!g.onPicture) return;
            if (g.held) {
                click(cursor.current.x, cursor.current.y, 2);
                return;
            }
        } else if (g.moved) {
            return;
        }
        // A tap: click. Two quick taps: double-click, where the first one was (fingers wobble).
        const last = lastTap.current;
        const double = !!last && now - last.t < 400 && (touchMode === 'trackpad' || Math.hypot(e.clientX - last.cx, e.clientY - last.cy) < 32);
        const at = double && last && touchMode === 'touch' ? last.at : { ...cursor.current };
        lastTap.current = double ? null : { t: now, cx: e.clientX, cy: e.clientY, at };
        click(at.x, at.y, 0, double ? 2 : 1);
    };

    // ---- the picture's pointer events: a mouse on the picture, fingers anywhere in the area ---

    /** Mouse buttons pressed on the picture: their release goes out wherever the mouse is then. */
    const mouseDown = useRef(new Set<number>());

    const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
        const c = clientRef.current;
        if (!canControl || !live || !c || isControl(e.target)) return;
        if (e.pointerType === 'touch') {
            onTouchDown(e, c);
            return;
        }
        if (e.target !== canvasRef.current) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        const p = norm(e.clientX, e.clientY);
        flushNow();
        mouseDown.current.add(e.button);
        c.pointer(p.x, p.y, 'move');
        c.pointer(p.x, p.y, 'down', e.button, Math.max(1, e.detail));
        e.preventDefault();
    };

    const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
        const c = clientRef.current;
        if (!canControl || !c) return;
        if (e.pointerType === 'touch') {
            onTouchMove(e);
            return;
        }
        if (e.target !== canvasRef.current && mouseDown.current.size === 0) return;
        if (!hovering.current && e.target === canvasRef.current) {
            hovering.current = true;
            paintCursor();
        }
        const p = clamp01(norm(e.clientX, e.clientY));
        sendMove(p.x, p.y);
    };

    const onPointerUp = (e: React.PointerEvent<HTMLDivElement>, cancelled = false) => {
        const c = clientRef.current;
        if (!canControl || !c) return;
        if (e.pointerType === 'touch') {
            onTouchUp(e, cancelled);
            return;
        }
        if (!mouseDown.current.delete(e.button)) return;
        const p = clamp01(norm(e.clientX, e.clientY));
        press(p.x, p.y, 'up', e.button, Math.max(1, e.detail));
    };

    // Touches on the picture mustn't turn into clicks or focus changes in this page: those
    // would close the phone's keyboard while typing. (React's touch listeners are passive.)
    useEffect(() => {
        const el = areaRef.current;
        if (!el) return;
        const stop = (e: TouchEvent) => {
            if (!isControl(e.target)) e.preventDefault();
        };
        el.addEventListener('touchstart', stop, { passive: false });

        return () => el.removeEventListener('touchstart', stop);
    }, []);

    // Wheel needs a non-passive listener to stop the page from scrolling.
    useEffect(() => {
        const el = canvasRef.current;
        if (!el) return;
        const onWheel = (e: WheelEvent) => {
            e.preventDefault();
            if (!canControl) return;
            const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? el.clientHeight : 1;
            clientRef.current?.wheel(e.deltaX * unit, e.deltaY * unit);
        };
        el.addEventListener('wheel', onWheel, { passive: false });

        return () => el.removeEventListener('wheel', onWheel);
    }, [canControl]);

    // ---- keyboard ------------------------------------------------------------------------

    const dialogOpen = clipText !== null || sheet;
    useEffect(() => {
        if (!live || !canControl || dialogOpen) return;
        const ours = (t: EventTarget | null) => t instanceof HTMLElement && (t.closest('.screen-bar, .screen-keys') !== null || t === kbRef.current);
        const onKey = (e: KeyboardEvent) => {
            if (ours(e.target)) return;
            if (!e.code || e.isComposing) return;
            e.preventDefault();
            e.stopPropagation();
            clientRef.current?.key(mapCode(e.code, swap), e.type === 'keydown', e.repeat);
        };
        const release = () => clientRef.current?.release();
        window.addEventListener('keydown', onKey, true);
        window.addEventListener('keyup', onKey, true);
        window.addEventListener('blur', release);
        document.addEventListener('visibilitychange', release);

        return () => {
            window.removeEventListener('keydown', onKey, true);
            window.removeEventListener('keyup', onKey, true);
            window.removeEventListener('blur', release);
            document.removeEventListener('visibilitychange', release);
            release();
        };
    }, [live, canControl, swap, dialogOpen]);

    /** A key from the on-screen bar, with any latched modifiers. */
    const tapKey = (code: string) => {
        clientRef.current?.combo(latched, code);
        setLatched([]);
    };

    /** Characters typed on a phone keyboard. */
    const typeChars = (text: string) => {
        const c = clientRef.current;
        if (!c || !text) return;
        if (latched.length === 0) {
            const parts = text.split('\n');
            parts.forEach((part, i) => {
                if (part) c.text(part);
                if (i < parts.length - 1) c.combo([], 'Enter');
            });
            return;
        }
        for (const ch of text) {
            const k = codeForChar(ch);
            if (k) c.combo([...latched, ...(k.shift ? ['ShiftLeft'] : [])], k.code);
            else c.text(ch);
        }
        setLatched([]);
    };

    const onKbInput = (e: FormEvent<HTMLInputElement>) => {
        const el = e.currentTarget;
        const v = el.value;
        if (v.length < KB_PAD.length) {
            for (let i = v.length; i < KB_PAD.length; i++) tapKey('Backspace');
        } else if (v.length > KB_PAD.length) {
            typeChars(v.slice(KB_PAD.length));
        }
        el.value = KB_PAD;
    };

    const onKbKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            tapKey('Enter');
        } else if (e.key.startsWith('Arrow') || e.key === 'Escape' || e.key === 'Tab') {
            e.preventDefault();
            tapKey(e.code || e.key);
        }
    };

    // Android's own "hide keyboard" leaves the field focused: then the button brings it back.
    const keyboardSeen = useRef(false);
    useEffect(() => {
        if (!keyboardOpen) keyboardSeen.current = false;
        else if (keyboardUp) keyboardSeen.current = true;
    }, [keyboardOpen, keyboardUp]);

    const toggleKeyboard = () => {
        const el = kbRef.current;
        if (!el) return;
        if (keyboardOpen && !(keyboardSeen.current && !keyboardUp)) {
            el.blur();
            setKeyboardOpen(false);
            return;
        }
        el.blur();
        el.value = KB_PAD;
        el.focus({ preventScroll: true });
        setKeyboardOpen(true);
    };

    // ---- clipboard -----------------------------------------------------------------------

    const sendClipboard = async () => {
        setMenu(null);
        setSheet(false);
        try {
            const text = await navigator.clipboard.readText();
            clientRef.current?.setClipboard(text);
        } catch {
            // No permission to read it (or not supported): type or paste it in a box.
            setClipText({ mode: 'to', text: '' });
        }
    };

    const getClipboard = () => {
        clientRef.current?.getClipboard();
        setMenu(null);
        setSheet(false);
    };

    const toggleFullscreen = async () => {
        try {
            if (document.fullscreenElement) {
                await document.exitFullscreen();
            } else {
                await rootRef.current?.requestFullscreen();
                // Chrome/Edge: also take keys like Esc and ⌘/Ctrl+W while full screen.
                await (navigator as Navigator & { keyboard?: { lock?: () => Promise<void> } }).keyboard?.lock?.();
            }
        } catch {
            // Not allowed here: ignore.
        }
    };

    const displays = hello?.displays ?? [];
    // Native only differs from Sharp on displays wider than 4K.
    const shown = displays.find((d) => d.id === display);
    // Beyond 4K needs HEVC, which only Macs encode.
    const nativeWide = shown && isMacTarget(hello?.os) && Math.round(shown.width * shown.scale) > 3840 ? Math.round(shown.width * shown.scale) : 0;
    const qualities = QUALITIES.filter((q) => q.value !== 'native' || nativeWide > 0 || quality === 'native');
    const swapLabel = hello && defaultSwap(hello.os) ? (isMacTarget(hello.os) ? 'Ctrl = ⌘' : '⌘ = Ctrl') : null;
    const swapTitle = isMacTarget(hello?.os) ? 'Use Ctrl as the Mac’s ⌘ key' : 'Use ⌘ as the computer’s Ctrl key';
    /** Buttons near the picture keep the phone keyboard's focus (a tap would close it). */
    const keepFocus = (e: React.PointerEvent) => e.preventDefault();
    const showKeys = canControl && touchDevice && (chrome || keyboardOpen);

    return (
        <div ref={rootRef} className="relative flex min-h-0 flex-1 flex-col bg-black">
            {chrome && (
                <Bar name={name} onClose={onClose}>
                    <StatusChip state={state} stats={stats} compact={compact} />
                    {compact ? (
                        <>
                            <span className="flex-1" />
                            {zoomed && (
                                <button type="button" className="hchip shrink-0 tabular-nums" onPointerDown={keepFocus} onClick={resetZoom} title="Back to the whole screen">
                                    {view.zoom.toFixed(1)}× ✕
                                </button>
                            )}
                            {canControl && touchDevice && (
                                <button type="button" className="hchip shrink-0" onPointerDown={keepFocus} onClick={() => setTouchMode(touchMode === 'trackpad' ? 'touch' : 'trackpad')} title="How touches move the pointer">
                                    {touchMode === 'trackpad' ? 'Trackpad' : 'Touch'}
                                </button>
                            )}
                            {canControl && touchDevice && (
                                <button type="button" className={cn('icon-btn', keyboardOpen && 'on')} onPointerDown={keepFocus} onClick={toggleKeyboard} aria-pressed={keyboardOpen} aria-label="Keyboard" title="Keyboard">
                                    <Glyph name="keyboard" />
                                </button>
                            )}
                            <button type="button" className="icon-btn" onPointerDown={keepFocus} onClick={() => setSheet(true)} aria-label="More" title="More">
                                <Glyph name="more" />
                            </button>
                        </>
                    ) : (
                        <>
                            {displays.length > 1 && (
                                <select className="hchip cursor-pointer bg-transparent" aria-label="Display" value={display ?? ''} onChange={(e) => clientRef.current?.display(Number(e.target.value))}>
                                    {displays.map((d) => (
                                        <option key={d.id} value={d.id}>
                                            {d.name}
                                            {d.primary ? ' (main)' : ''}
                                        </option>
                                    ))}
                                </select>
                            )}
                            <select className="hchip cursor-pointer bg-transparent" aria-label="Picture quality" value={quality} onChange={(e) => clientRef.current?.quality(e.target.value as Quality)}>
                                {qualities.map((q) => (
                                    <option key={q.value} value={q.value}>
                                        {q.value === 'native' && nativeWide ? `Native (${nativeWide} px, fewer fps)` : q.label}
                                    </option>
                                ))}
                            </select>
                            {!touchDevice && (
                                <button type="button" className={cn('hchip', fit === 'actual' && 'on')} aria-pressed={fit === 'actual'} onClick={() => setFit(fit === 'fit' ? 'actual' : 'fit')} title="Show one screen pixel per pixel">
                                    {fit === 'fit' ? 'Fit' : '1:1'}
                                </button>
                            )}
                            {touchDevice && (
                                <button type="button" className="hchip" onPointerDown={keepFocus} onClick={() => setTouchMode(touchMode === 'trackpad' ? 'touch' : 'trackpad')} title="How touches move the pointer">
                                    {touchMode === 'trackpad' ? 'Trackpad' : 'Touch'}
                                </button>
                            )}
                            {touchDevice && zoomed && (
                                <button type="button" className="hchip" onPointerDown={keepFocus} onClick={resetZoom}>
                                    Zoom {view.zoom.toFixed(1)}× · reset
                                </button>
                            )}
                            {canControl && touchDevice && (
                                <button type="button" className={cn('hchip', keyboardOpen && 'on')} aria-pressed={keyboardOpen} onPointerDown={keepFocus} onClick={toggleKeyboard}>
                                    Keyboard
                                </button>
                            )}
                            {canControl && swapLabel && (
                                <button type="button" className={cn('hchip', swap && 'on')} aria-pressed={swap} onClick={() => setSwap(!swap)} title={swapTitle}>
                                    {swapLabel}
                                </button>
                            )}
                            {canControl && (
                                <Menu label="Keys" open={menu === 'keys'} onOpen={(o) => setMenu(o ? 'keys' : null)}>
                                    {shortcuts(hello?.os).map((s) => (
                                        <button key={s.label} type="button" className="menu-item" onClick={() => { clientRef.current?.combo(s.mods, s.code); setMenu(null); }}>
                                            {s.label}
                                        </button>
                                    ))}
                                </Menu>
                            )}
                            <Menu label="Clipboard" open={menu === 'clipboard'} onOpen={(o) => setMenu(o ? 'clipboard' : null)}>
                                <button type="button" className="menu-item" onClick={() => void sendClipboard()}>
                                    Send my clipboard to the computer
                                </button>
                                <button type="button" className="menu-item" onClick={getClipboard}>
                                    Get the computer’s clipboard
                                </button>
                            </Menu>
                            {canFullscreen && (
                                <button type="button" className="icon-btn ml-auto" onClick={() => void toggleFullscreen()} aria-label={fullscreen ? 'Leave full screen' : 'Full screen'} title={fullscreen ? 'Leave full screen' : 'Full screen'}>
                                    <Glyph name={fullscreen ? 'shrink' : 'expand'} />
                                </button>
                            )}
                        </>
                    )}
                </Bar>
            )}

            {chrome && hello && !hello.input && live && (
                <p className="m-0 shrink-0 bg-amber-bg px-3 py-1.5 text-center text-[13px] text-amber-soft">
                    View only: Termlab on this computer isn’t allowed to control it. On the computer, allow Termlab under System Settings › Privacy & Security › Accessibility.
                </p>
            )}

            <div
                ref={areaRef}
                className={cn('relative min-h-0 flex-1 touch-none select-none', fit === 'actual' ? 'overflow-auto' : 'overflow-hidden')}
                onPointerDown={onPointerDown}
                onPointerMove={onPointerMove}
                onPointerUp={(e) => onPointerUp(e)}
                onPointerCancel={(e) => onPointerUp(e, true)}
                onContextMenu={(e) => e.preventDefault()}
            >
                <canvas
                    ref={canvasRef}
                    width={16}
                    height={9}
                    className={cn('screen-canvas absolute', !live && 'opacity-0')}
                    style={layout ? { width: layout.w, height: layout.h, left: layout.left, top: layout.top } : { width: 0, height: 0 }}
                    onPointerEnter={onPointerEnter}
                    onPointerLeave={onPointerLeave}
                />
                {/* The computer's pointer, when the browser's own isn't over the picture. */}
                <canvas ref={overlayRef} aria-hidden className="pointer-events-none absolute top-0 left-0 hidden origin-top-left" />
                {!chrome && (
                    <button
                        type="button"
                        className="icon-btn absolute top-2 right-2 z-10 border-line bg-[rgba(15,17,19,0.72)] text-ink"
                        onPointerDown={keepFocus}
                        onClick={() => setChrome(true)}
                        aria-label="Show the toolbar"
                        title="Show the toolbar"
                    >
                        <Glyph name="bars" />
                    </button>
                )}
                {state.state !== 'live' && (
                    <div className="absolute inset-0 grid place-items-center p-4">
                        <Card>
                            {state.state === 'connecting' && <p className="m-0 text-ink-soft">Connecting to {name}…</p>}
                            {state.state === 'confirming' && <p className="m-0 text-ink-soft">Waiting for someone at {name} to allow it…</p>}
                            {state.state === 'ended' && (
                                <>
                                    <p className="m-0 mb-4 text-ink-soft">{state.message ?? 'The screen session ended.'}</p>
                                    <div className="flex justify-center gap-2">
                                        <Button variant="outline" onClick={onClose}>
                                            Close
                                        </Button>
                                        <Button onClick={onRestart}>Reconnect</Button>
                                    </div>
                                </>
                            )}
                        </Card>
                    </div>
                )}
            </div>

            {/* The phone keyboard types into this; it stays out of sight. 16px, or iPhones zoom in on it. */}
            <input
                ref={kbRef}
                aria-label="Type on the computer"
                className="pointer-events-none fixed bottom-0 left-0 h-px w-px text-[16px] opacity-0"
                autoCapitalize="off"
                autoCorrect="off"
                autoComplete="off"
                spellCheck={false}
                defaultValue={KB_PAD}
                onInput={onKbInput}
                onKeyDown={onKbKeyDown}
                onBlur={() => setKeyboardOpen(false)}
            />

            {showKeys && (
                <div className={cn('screen-keys flex shrink-0 gap-1 overflow-x-auto border-t border-line bg-surface px-1.5 py-1.5 [scrollbar-width:none]', !keyboardUp && 'pb-[max(6px,env(safe-area-inset-bottom))]')}>
                    {modifiers(hello?.os).map((m) => {
                        const on = latched.includes(m.code);

                        return (
                            <button
                                key={m.code}
                                type="button"
                                aria-pressed={on}
                                className={cn('keycap', on && 'on')}
                                onPointerDown={keepFocus}
                                onClick={() => setLatched(on ? latched.filter((c) => c !== m.code) : [...latched, m.code])}
                            >
                                {m.label}
                            </button>
                        );
                    })}
                    {KEYBAR.map((k) => (
                        <button key={k.code} type="button" className="keycap" title={k.title} onPointerDown={keepFocus} onClick={() => tapKey(k.code)}>
                            {k.label}
                        </button>
                    ))}
                </div>
            )}

            <Sheet open={sheet} onClose={() => setSheet(false)} title="Screen options">
                <SheetSection title="Picture">
                    <div className="segmented" role="radiogroup" aria-label="Picture quality">
                        {qualities.map((q) => (
                            <button key={q.value} type="button" role="radio" aria-checked={quality === q.value} className={cn('flex-1', quality === q.value && 'on')} onClick={() => clientRef.current?.quality(q.value)}>
                                {q.short}
                            </button>
                        ))}
                    </div>
                    {nativeWide > 0 && <p className="m-0 text-[12px] text-dim">Native shows all {nativeWide} pixels, at fewer frames per second.</p>}
                    {displays.length > 1 && (
                        <select className={inputClass} aria-label="Display" value={display ?? ''} onChange={(e) => clientRef.current?.display(Number(e.target.value))}>
                            {displays.map((d) => (
                                <option key={d.id} value={d.id}>
                                    {d.name}
                                    {d.primary ? ' (main)' : ''}
                                </option>
                            ))}
                        </select>
                    )}
                    {!touchDevice && (
                        <div className="segmented" role="radiogroup" aria-label="Size">
                            <button type="button" role="radio" aria-checked={fit === 'fit'} className={cn('flex-1', fit === 'fit' && 'on')} onClick={() => setFit('fit')}>
                                Fit to window
                            </button>
                            <button type="button" role="radio" aria-checked={fit === 'actual'} className={cn('flex-1', fit === 'actual' && 'on')} onClick={() => setFit('actual')}>
                                1:1 pixels
                            </button>
                        </div>
                    )}
                    {zoomed && (
                        <button type="button" className="menu-item" onClick={() => { resetZoom(); setSheet(false); }}>
                            Back to the whole screen (now {view.zoom.toFixed(1)}×)
                        </button>
                    )}
                </SheetSection>

                {canControl && touchDevice && (
                    <SheetSection title="Touch">
                        <div className="segmented" role="radiogroup" aria-label="Touch mode">
                            {(['trackpad', 'touch'] as const).map((m) => (
                                <button key={m} type="button" role="radio" aria-checked={touchMode === m} className={cn('flex-1', touchMode === m && 'on')} onClick={() => setTouchMode(m)}>
                                    {m === 'trackpad' ? 'Trackpad' : 'Touch'}
                                </button>
                            ))}
                        </div>
                        <dl className="m-0 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[13px]">
                            {GESTURES[touchMode].map(([g, what]) => (
                                <div key={g} className="contents">
                                    <dt className="text-ink-soft">{g}</dt>
                                    <dd className="m-0 text-dim">{what}</dd>
                                </div>
                            ))}
                        </dl>
                    </SheetSection>
                )}

                {canControl && (
                    <SheetSection title="Keys">
                        {swapLabel && (
                            <label className="flex cursor-pointer items-center justify-between gap-3 py-1">
                                <span className="text-ink-soft">{swapTitle}</span>
                                <input type="checkbox" className="switch" checked={swap} onChange={(e) => setSwap(e.target.checked)} />
                            </label>
                        )}
                        <div className="flex flex-col">
                            {shortcuts(hello?.os).map((s) => (
                                <button key={s.label} type="button" className="menu-item" onClick={() => { clientRef.current?.combo(s.mods, s.code); setSheet(false); }}>
                                    {s.label}
                                </button>
                            ))}
                        </div>
                    </SheetSection>
                )}

                <SheetSection title="Clipboard">
                    <div className="flex flex-col">
                        <button type="button" className="menu-item" onClick={() => void sendClipboard()}>
                            Send my clipboard to the computer
                        </button>
                        <button type="button" className="menu-item" onClick={getClipboard}>
                            Get the computer’s clipboard
                        </button>
                    </div>
                </SheetSection>

                <SheetSection title="View">
                    <div className="flex flex-col">
                        {canFullscreen && (
                            <button type="button" className="menu-item" onClick={() => { setSheet(false); void toggleFullscreen(); }}>
                                {fullscreen ? 'Leave full screen' : 'Full screen'}
                            </button>
                        )}
                        <button type="button" className="menu-item" onClick={() => { setChrome(false); setSheet(false); }}>
                            Hide the toolbar (the button top right brings it back)
                        </button>
                    </div>
                </SheetSection>
            </Sheet>

            <Dialog
                open={clipText !== null}
                onClose={() => setClipText(null)}
                title={clipText?.mode === 'from' ? 'The computer’s clipboard' : 'Send text to the computer’s clipboard'}
                footer={
                    <div className="flex justify-end gap-2">
                        <Button variant="outline" onClick={() => setClipText(null)}>
                            Close
                        </Button>
                        {clipText?.mode === 'from' ? (
                            <Button
                                onClick={() => {
                                    void navigator.clipboard.writeText(clipText.text).then(
                                        () => toast.success('Copied.'),
                                        () => toast.error('Couldn’t copy: select the text and copy it.'),
                                    );
                                }}
                            >
                                Copy
                            </Button>
                        ) : (
                            <Button
                                onClick={() => {
                                    clientRef.current?.setClipboard(clipText?.text ?? '');
                                    setClipText(null);
                                }}
                            >
                                Send
                            </Button>
                        )}
                    </div>
                }
            >
                <textarea
                    className={`${inputClass} min-h-40 w-full font-mono text-[13px]`}
                    readOnly={clipText?.mode === 'from'}
                    value={clipText?.text ?? ''}
                    placeholder={clipText?.mode === 'to' ? 'Paste or type the text here' : undefined}
                    onChange={(e) => setClipText((c) => (c ? { ...c, text: e.target.value } : c))}
                />
            </Dialog>
        </div>
    );
}

function StatusChip({ state, stats, compact }: { state: ScreenState; stats: ScreenStats | null; compact: boolean }) {
    const color = state.state === 'live' ? '#3FB950' : state.state === 'ended' ? '#8B929C' : '#F5A524';
    const rate = stats ? (stats.kbps >= 1000 ? `${(stats.kbps / 1000).toFixed(1)} Mbit/s` : `${stats.kbps} kbit/s`) : '';
    const label =
        state.state === 'live'
            ? stats
                ? compact
                    ? `${stats.fps} fps`
                    : `${stats.fps} fps · ${rate}`
                : 'Live'
            : state.state === 'ended'
              ? 'Ended'
              : state.state === 'confirming'
                ? compact
                    ? 'Waiting'
                    : 'Waiting for approval'
                : 'Connecting';

    return (
        <span className="hchip shrink-0 tabular-nums" title={stats && stats.width ? `${stats.codec} ${stats.width}×${stats.height} · ${rate}` : undefined}>
            <span aria-hidden="true" className="dot" style={{ background: color }} />
            {label}
        </span>
    );
}

function Menu({ label, open, onOpen, children }: { label: string; open: boolean; onOpen: (open: boolean) => void; children: ReactNode }) {
    const ref = useRef<HTMLDivElement>(null);
    useEffect(() => {
        if (!open) return;
        const away = (e: PointerEvent) => {
            if (!ref.current?.contains(e.target as Node)) onOpen(false);
        };
        document.addEventListener('pointerdown', away);

        return () => document.removeEventListener('pointerdown', away);
    }, [open, onOpen]);

    return (
        <div ref={ref} className="shrink-0">
            <button type="button" className={cn('hchip', open && 'on')} aria-expanded={open} onClick={() => onOpen(!open)}>
                {label}
                <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                    <path d="m6 9 6 6 6-6" />
                </svg>
            </button>
            {open && <div className="screen-menu fixed z-[70] mt-1 flex min-w-[240px] flex-col rounded-lg border border-line bg-panel p-1 shadow-2xl">{children}</div>}
        </div>
    );
}

/** A panel from the bottom with the controls that don't fit a phone's bar. */
function Sheet({ open, onClose, title, children }: { open: boolean; onClose: () => void; title: string; children: ReactNode }) {
    const box = useRef<HTMLDivElement>(null);
    useEffect(() => {
        if (!open) return;
        box.current?.focus({ preventScroll: true });
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                onClose();
            }
        };
        document.addEventListener('keydown', onKey);

        return () => document.removeEventListener('keydown', onKey);
    }, [open, onClose]);

    if (!open) return null;

    return (
        <div
            className="absolute inset-0 z-[70] flex items-end justify-center bg-[rgba(4,5,6,0.6)]"
            onClick={(e) => {
                if (e.target === e.currentTarget) onClose();
            }}
        >
            <div
                ref={box}
                role="dialog"
                aria-modal="true"
                aria-label={title}
                tabIndex={-1}
                className="screen-sheet flex max-h-[85%] w-full max-w-[520px] flex-col gap-4 overflow-y-auto overscroll-contain rounded-t-2xl border border-b-0 border-line-strong bg-panel px-4 pt-2 pb-[max(16px,env(safe-area-inset-bottom))] shadow-2xl outline-none"
            >
                <div className="relative flex items-center justify-center pt-1">
                    <span aria-hidden="true" className="h-1 w-10 rounded-full bg-line-strong" />
                    <button type="button" className="icon-btn absolute top-0 right-[-8px]" onClick={onClose} aria-label="Close" title="Close">
                        <Icon name="close" size={16} />
                    </button>
                </div>
                {children}
            </div>
        </div>
    );
}

function SheetSection({ title, children }: { title: string; children: ReactNode }) {
    return (
        <section className="flex flex-col gap-2">
            <h3 className="section-heading">{title}</h3>
            {children}
        </section>
    );
}
