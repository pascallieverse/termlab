import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from 'react';
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

const KEYBAR: { label: string; code: string; title: string }[] = [
    { label: 'Esc', code: 'Escape', title: 'Escape' },
    { label: 'Tab', code: 'Tab', title: 'Tab' },
    { label: '←', code: 'ArrowLeft', title: 'Left' },
    { label: '↑', code: 'ArrowUp', title: 'Up' },
    { label: '↓', code: 'ArrowDown', title: 'Down' },
    { label: '→', code: 'ArrowRight', title: 'Right' },
    { label: '⌫', code: 'Backspace', title: 'Delete' },
    { label: '⏎', code: 'Enter', title: 'Return' },
];

/** Keep this much text in the hidden phone-keyboard field, so Backspace has something to delete. */
const KB_PAD = '  ';

/** Browsers don't show cursor images larger than this (CSS pixels). */
const MAX_CSS_CURSOR = 128;

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
        <div className="screen-viewer fixed inset-0 z-[60] flex flex-col bg-black text-sm text-ink" role="dialog" aria-modal="true" aria-label={`Screen of ${name}`}>
            {phase.kind === 'session' ? (
                <Session key={phase.sid} sid={phase.sid} name={name} onClose={close} onRestart={() => void begin()} toast={toast} />
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
            <span className="flex min-w-0 shrink-0 items-center gap-1.5 font-medium">
                <Icon name="screen" size={16} />
                <span className="max-w-[160px] truncate">{name}</span>
            </span>
            {children}
        </header>
    );
}

type Toast = ReturnType<typeof useToast>;

function Session({ sid, name, onClose, onRestart, toast }: { sid: string; name: string; onClose: () => void; onRestart: () => void; toast: Toast }) {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const areaRef = useRef<HTMLDivElement>(null);
    const rootRef = useRef<HTMLDivElement>(null);
    const kbRef = useRef<HTMLInputElement>(null);
    const clientRef = useRef<ScreenClient | null>(null);
    const [state, setState] = useState<ScreenState>({ state: 'connecting' });
    const [hello, setHello] = useState<ScreenHello | null>(null);
    const [display, setDisplay] = useState<number | null>(null);
    const [quality, setQuality] = useState<Quality>('balanced');
    const [video, setVideo] = useState<{ w: number; h: number } | null>(null);
    const [area, setArea] = useState<{ w: number; h: number }>({ w: 0, h: 0 });
    const [stats, setStats] = useState<ScreenStats | null>(null);
    const [fit, setFit] = useState<Fit>('fit');
    const [zoom, setZoom] = useState(1);
    const [pan, setPan] = useState({ x: 0, y: 0 });
    const touchDevice = useMemo(() => matchMedia('(pointer: coarse)').matches, []);
    const [touchMode, setTouchMode] = useState<TouchMode>('trackpad');
    const [swap, setSwap] = useState(false);
    const [latched, setLatched] = useState<string[]>([]);
    const [keyboardOpen, setKeyboardOpen] = useState(false);
    const [menu, setMenu] = useState<'keys' | 'clipboard' | null>(null);
    const [clipText, setClipText] = useState<{ mode: 'from' | 'to'; text: string } | null>(null);
    const [fullscreen, setFullscreen] = useState(false);
    const canControl = hello?.input ?? false;
    const live = state.state === 'live';
    const overlayRef = useRef<HTMLCanvasElement>(null);
    /** The computer's pointer, as last heard (positions from here while a finger moves it). */
    const remote = useRef<RemoteCursor | null>(null);
    /** A mouse is over the picture: the browser's own pointer shows the computer's shape. */
    const hovering = useRef(false);
    const localUntil = useRef(0);
    const cursorCache = useRef(new Map<string, string[]>());
    const view = useRef<{ layout: { w: number; h: number; left: number; top: number } | null; pointsWide: number; control: boolean; live: boolean }>({
        layout: null,
        pointsWide: 0,
        control: false,
        live: false,
    });

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
        const s = Math.min(area.w / video.w, area.h / video.h) * zoom;
        const w = video.w * s;
        const h = video.h * s;
        const clamp = (offset: number, size: number, room: number) => (size <= room ? (room - size) / 2 : Math.min(0, Math.max(room - size, offset)));

        return { w, h, left: clamp((area.w - w) / 2 + pan.x, w, area.w), top: clamp((area.h - h) / 2 + pan.y, h, area.h) };
    }, [video, area, fit, zoom, pan]);

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
        const { layout: l, pointsWide, control, live: on } = view.current;
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
        const sx = w / shape.width;
        const sy = h / shape.height;
        ov.style.display = 'block';
        ov.style.width = `${w}px`;
        ov.style.height = `${h}px`;
        ov.style.transform = `translate(${l.left + c.x * l.w - shape.hotX * sx}px, ${l.top + c.y * l.h - shape.hotY * sy}px)`;
    }, []);

    const pointsWide = hello?.displays.find((d) => d.id === display)?.width ?? 0;
    useLayoutEffect(() => {
        view.current = { layout, pointsWide, control: canControl, live };
        paintCursor();
    }, [layout, pointsWide, canControl, live, paintCursor]);

    /** Moved here (a finger, in trackpad mode): show it there before the computer says so. */
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

    // ---- mouse ---------------------------------------------------------------------------

    const norm = (clientX: number, clientY: number) => {
        const r = canvasRef.current!.getBoundingClientRect();

        return { x: (clientX - r.left) / r.width, y: (clientY - r.top) / r.height };
    };

    const pendingMove = useRef<{ x: number; y: number } | null>(null);
    const moveFrame = useRef(0);
    const sendMove = (x: number, y: number) => {
        pendingMove.current = { x, y };
        if (!moveFrame.current) {
            moveFrame.current = requestAnimationFrame(() => {
                moveFrame.current = 0;
                const p = pendingMove.current;
                if (p) clientRef.current?.pointer(p.x, p.y, 'move');
            });
        }
    };

    // ---- touch: trackpad (relative) or direct, two fingers scroll, pinch zooms ----------

    const cursor = useRef({ x: 0.5, y: 0.5 });
    const touches = useRef(new Map<number, { x: number; y: number; sx: number; sy: number; t: number }>());
    const gesture = useRef<{ moved: boolean; dragging: boolean; hold: number; two: boolean; dist: number; zoomed: boolean; lastTap: number } | null>(null);
    const lastTapAt = useRef(0);

    /** Keeps the cursor on screen when zoomed in (trackpad mode). */
    const follow = (nx: number, ny: number) => {
        if (!layout || zoom <= 1 || fit !== 'fit') return;
        const px = layout.left + nx * layout.w;
        const py = layout.top + ny * layout.h;
        const margin = 40;
        let dx = 0;
        let dy = 0;
        if (px < margin) dx = margin - px;
        if (px > area.w - margin) dx = area.w - margin - px;
        if (py < margin) dy = margin - py;
        if (py > area.h - margin) dy = area.h - margin - py;
        if (dx || dy) setPan((p) => ({ x: p.x + dx, y: p.y + dy }));
    };

    const click = (x: number, y: number, button: number, clicks = 1) => {
        const c = clientRef.current;
        if (!c) return;
        c.pointer(x, y, 'move');
        c.pointer(x, y, 'down', button, clicks);
        c.pointer(x, y, 'up', button, clicks);
    };

    const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
        if (!canControl) return;
        const c = clientRef.current;
        if (!c) return;
        if (e.pointerType === 'mouse' || e.pointerType === 'pen') {
            e.currentTarget.setPointerCapture(e.pointerId);
            const p = norm(e.clientX, e.clientY);
            c.pointer(p.x, p.y, 'move');
            c.pointer(p.x, p.y, 'down', e.button, Math.max(1, e.detail));
            e.preventDefault();
            return;
        }
        // Touch.
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        touches.current.set(e.pointerId, { x: e.clientX, y: e.clientY, sx: e.clientX, sy: e.clientY, t: performance.now() });
        const n = touches.current.size;
        if (n === 1) {
            const g = { moved: false, dragging: false, hold: 0, two: false, dist: 0, zoomed: false, lastTap: 0 };
            if (touchMode === 'trackpad') {
                g.hold = window.setTimeout(() => {
                    // Press and hold, then move: drag.
                    if (!g.moved) {
                        g.dragging = true;
                        c.pointer(cursor.current.x, cursor.current.y, 'down', 0, 1);
                        navigator.vibrate?.(10);
                    }
                }, 450);
            } else {
                const p = norm(e.clientX, e.clientY);
                cursor.current = p;
                c.pointer(p.x, p.y, 'move');
            }
            gesture.current = g;
        } else if (n === 2 && gesture.current) {
            window.clearTimeout(gesture.current.hold);
            gesture.current.two = true;
            const [a, b] = [...touches.current.values()];
            gesture.current.dist = Math.hypot(a.x - b.x, a.y - b.y);
        }
    };

    const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
        if (!canControl) return;
        const c = clientRef.current;
        if (!c) return;
        if (e.pointerType === 'mouse' || e.pointerType === 'pen') {
            if (!hovering.current) {
                hovering.current = true;
                paintCursor();
            }
            const p = norm(e.clientX, e.clientY);
            sendMove(p.x, p.y);
            return;
        }
        const t = touches.current.get(e.pointerId);
        const g = gesture.current;
        if (!t || !g) return;
        const dx = e.clientX - t.x;
        const dy = e.clientY - t.y;
        t.x = e.clientX;
        t.y = e.clientY;
        if (Math.hypot(e.clientX - t.sx, e.clientY - t.sy) > 8) {
            if (!g.moved && !g.dragging) window.clearTimeout(g.hold);
            g.moved = true;
        }
        if (touches.current.size >= 2) {
            const [a, b] = [...touches.current.values()];
            const dist = Math.hypot(a.x - b.x, a.y - b.y);
            if (fit === 'fit' && (g.zoomed || Math.abs(dist - g.dist) > 24)) {
                // Pinch: zoom the picture here (not on the computer).
                g.zoomed = true;
                const ratio = dist / Math.max(1, g.dist);
                g.dist = dist;
                setZoom((z) => Math.min(6, Math.max(1, z * ratio)));
                setPan((p) => ({ x: p.x + dx / 2, y: p.y + dy / 2 }));
            } else if (e.pointerId === [...touches.current.keys()][0]) {
                // Two fingers: scroll (natural, as on a trackpad).
                c.wheel(-dx * 2, -dy * 2);
            }
            return;
        }
        if (touchMode === 'trackpad') {
            if (!g.moved && !g.dragging) return;
            const r = canvasRef.current!.getBoundingClientRect();
            const speed = 1.6;
            cursor.current = {
                x: Math.min(1, Math.max(0, cursor.current.x + (dx * speed) / r.width)),
                y: Math.min(1, Math.max(0, cursor.current.y + (dy * speed) / r.height)),
            };
            sendMove(cursor.current.x, cursor.current.y);
            movedLocally(cursor.current.x, cursor.current.y);
            follow(cursor.current.x, cursor.current.y);
        } else {
            const p = norm(e.clientX, e.clientY);
            if (!g.dragging && g.moved) {
                g.dragging = true;
                c.pointer(cursor.current.x, cursor.current.y, 'down', 0, 1);
            }
            cursor.current = p;
            sendMove(p.x, p.y);
        }
    };

    const onPointerUp = (e: React.PointerEvent<HTMLCanvasElement>) => {
        if (!canControl) return;
        const c = clientRef.current;
        if (!c) return;
        if (e.pointerType === 'mouse' || e.pointerType === 'pen') {
            const p = norm(e.clientX, e.clientY);
            c.pointer(p.x, p.y, 'up', e.button, Math.max(1, e.detail));
            return;
        }
        const t = touches.current.get(e.pointerId);
        touches.current.delete(e.pointerId);
        const g = gesture.current;
        if (!t || !g) return;
        if (touches.current.size > 0) return;
        window.clearTimeout(g.hold);
        gesture.current = null;
        if (g.dragging) {
            c.pointer(cursor.current.x, cursor.current.y, 'up', 0, 1);
            return;
        }
        if (g.moved || g.zoomed) return;
        if (g.two) {
            click(cursor.current.x, cursor.current.y, 2);
            return;
        }
        // A tap: click (two quick taps: double click).
        const now = performance.now();
        const double = now - lastTapAt.current < 350;
        lastTapAt.current = double ? 0 : now;
        if (touchMode === 'touch') {
            cursor.current = norm(e.clientX, e.clientY);
        }
        click(cursor.current.x, cursor.current.y, 0, double ? 2 : 1);
    };

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

    const dialogOpen = clipText !== null;
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

    const openKeyboard = () => {
        const el = kbRef.current;
        if (!el) return;
        if (keyboardOpen) {
            el.blur();
            setKeyboardOpen(false);
            return;
        }
        el.value = KB_PAD;
        el.focus();
        setKeyboardOpen(true);
    };

    // ---- clipboard -----------------------------------------------------------------------

    const sendClipboard = async () => {
        setMenu(null);
        try {
            const text = await navigator.clipboard.readText();
            clientRef.current?.setClipboard(text);
        } catch {
            // No permission to read it (or not supported): type or paste it in a box.
            setClipText({ mode: 'to', text: '' });
        }
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
            // Not allowed here (e.g. iPhone): ignore.
        }
    };

    const displays = hello?.displays ?? [];
    // Native only differs from Sharp on displays wider than 4K.
    const shown = displays.find((d) => d.id === display);
    // Beyond 4K needs HEVC, which only Macs encode.
    const nativeWide = shown && isMacTarget(hello?.os) && Math.round(shown.width * shown.scale) > 3840 ? Math.round(shown.width * shown.scale) : 0;

    return (
        <div ref={rootRef} className="flex min-h-0 flex-1 flex-col bg-black">
            <Bar name={name} onClose={onClose}>
                <StatusChip state={state} stats={stats} />
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
                    <option value="low">Low (data saver)</option>
                    <option value="balanced">Balanced</option>
                    <option value="high">Sharp (4K)</option>
                    {(nativeWide > 0 || quality === 'native') && <option value="native">Native{nativeWide ? ` (${nativeWide} px, fewer fps)` : ''}</option>}
                </select>
                {!touchDevice && (
                    <button type="button" className={cn('hchip', fit === 'actual' && 'on')} aria-pressed={fit === 'actual'} onClick={() => setFit(fit === 'fit' ? 'actual' : 'fit')} title="Show one screen pixel per pixel">
                        {fit === 'fit' ? 'Fit' : '1:1'}
                    </button>
                )}
                {touchDevice && (
                    <button type="button" className="hchip" onClick={() => setTouchMode(touchMode === 'trackpad' ? 'touch' : 'trackpad')} title="How touches move the pointer">
                        {touchMode === 'trackpad' ? 'Trackpad' : 'Touch'}
                    </button>
                )}
                {touchDevice && zoom > 1 && (
                    <button type="button" className="hchip" onClick={() => { setZoom(1); setPan({ x: 0, y: 0 }); }}>
                        Zoom {zoom.toFixed(1)}× · reset
                    </button>
                )}
                {canControl && touchDevice && (
                    <button type="button" className={cn('hchip', keyboardOpen && 'on')} aria-pressed={keyboardOpen} onClick={openKeyboard}>
                        Keyboard
                    </button>
                )}
                {canControl && hello && defaultSwap(hello.os) && (
                    <button
                        type="button"
                        className={cn('hchip', swap && 'on')}
                        aria-pressed={swap}
                        onClick={() => setSwap(!swap)}
                        title={isMacTarget(hello.os) ? 'Use Ctrl as the Mac’s ⌘ key' : 'Use ⌘ as the computer’s Ctrl key'}
                    >
                        {isMacTarget(hello.os) ? 'Ctrl = ⌘' : '⌘ = Ctrl'}
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
                    <button type="button" className="menu-item" onClick={() => { clientRef.current?.getClipboard(); setMenu(null); }}>
                        Get the computer’s clipboard
                    </button>
                </Menu>
                {!touchDevice && (
                    <button type="button" className="icon-btn ml-auto" onClick={() => void toggleFullscreen()} aria-label={fullscreen ? 'Leave full screen' : 'Full screen'} title={fullscreen ? 'Leave full screen' : 'Full screen'}>
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
                            {fullscreen ? <path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5" /> : <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />}
                        </svg>
                    </button>
                )}
            </Bar>

            {hello && !hello.input && live && (
                <p className="m-0 shrink-0 bg-amber-bg px-3 py-1.5 text-center text-[13px] text-amber-soft">
                    View only: Termlab on this computer isn’t allowed to control it. On the computer, allow Termlab under System Settings › Privacy & Security › Accessibility.
                </p>
            )}

            <div ref={areaRef} className={cn('relative min-h-0 flex-1', fit === 'actual' ? 'overflow-auto' : 'overflow-hidden')}>
                <canvas
                    ref={canvasRef}
                    width={16}
                    height={9}
                    className={cn('screen-canvas absolute touch-none select-none', !live && 'opacity-0')}
                    style={layout ? { width: layout.w, height: layout.h, left: layout.left, top: layout.top } : { width: 0, height: 0 }}
                    onPointerEnter={onPointerEnter}
                    onPointerLeave={onPointerLeave}
                    onPointerDown={onPointerDown}
                    onPointerMove={onPointerMove}
                    onPointerUp={onPointerUp}
                    onPointerCancel={onPointerUp}
                    onContextMenu={(e) => e.preventDefault()}
                />
                {/* The computer's pointer, when the browser's own isn't over the picture. */}
                <canvas ref={overlayRef} aria-hidden className="pointer-events-none absolute top-0 left-0 hidden origin-top-left" />
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

            {/* The phone keyboard types into this; it stays out of sight. */}
            <input
                ref={kbRef}
                aria-label="Type on the computer"
                className="pointer-events-none fixed bottom-0 left-0 h-px w-px opacity-0"
                autoCapitalize="off"
                autoCorrect="off"
                autoComplete="off"
                spellCheck={false}
                defaultValue={KB_PAD}
                onInput={onKbInput}
                onKeyDown={onKbKeyDown}
                onBlur={() => setKeyboardOpen(false)}
            />

            {canControl && touchDevice && (
                <div className="screen-keys flex shrink-0 gap-1 overflow-x-auto border-t border-line bg-surface px-1.5 py-1.5 pb-[max(6px,env(safe-area-inset-bottom))] [scrollbar-width:none]">
                    {modifiers(hello?.os).map((m) => {
                        const on = latched.includes(m.code);

                        return (
                            <button
                                key={m.code}
                                type="button"
                                aria-pressed={on}
                                className={cn('keycap', on && 'on')}
                                onPointerDown={(e) => e.preventDefault()}
                                onClick={() => setLatched(on ? latched.filter((c) => c !== m.code) : [...latched, m.code])}
                            >
                                {m.label}
                            </button>
                        );
                    })}
                    {KEYBAR.map((k) => (
                        <button key={k.code} type="button" className="keycap" title={k.title} onPointerDown={(e) => e.preventDefault()} onClick={() => tapKey(k.code)}>
                            {k.label}
                        </button>
                    ))}
                </div>
            )}

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

function StatusChip({ state, stats }: { state: ScreenState; stats: ScreenStats | null }) {
    const color = state.state === 'live' ? '#3FB950' : state.state === 'ended' ? '#8B929C' : '#F5A524';
    const label =
        state.state === 'live'
            ? stats
                ? `${stats.fps} fps · ${stats.kbps >= 1000 ? `${(stats.kbps / 1000).toFixed(1)} Mbit/s` : `${stats.kbps} kbit/s`}`
                : 'Live'
            : state.state === 'ended'
              ? 'Ended'
              : state.state === 'confirming'
                ? 'Waiting for approval'
                : 'Connecting';

    return (
        <span className="hchip shrink-0 tabular-nums" title={stats && stats.width ? `${stats.codec} ${stats.width}×${stats.height}` : undefined}>
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
