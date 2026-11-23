/**
 * One screen session in the browser: the websocket to the server, the H.264 decoder
 * (WebCodecs) and the canvas it draws on. Input goes the other way as small JSON messages.
 *
 * Flow control: every decoded frame is acknowledged; the computer sends at most a few frames
 * ahead, so a slow connection gets fewer frames instead of a growing delay.
 * The wire format is in docs/server-protocol.md ("Screen sessions").
 */

export type Quality = 'low' | 'balanced' | 'high';

export type ScreenDisplay = { id: number; name: string; width: number; height: number; scale: number; primary: boolean };

export type ScreenHello = {
    displays: ScreenDisplay[];
    display: number;
    quality: Quality;
    /** The computer accepts mouse and keyboard input (macOS: Accessibility allowed). */
    input: boolean;
    os: string;
    version: string;
};

export type ScreenState =
    | { state: 'connecting' }
    | { state: 'confirming' }
    | { state: 'live' }
    | { state: 'ended'; message: string | null };

export type ScreenStats = { fps: number; kbps: number; width: number; height: number };

type Handlers = {
    onState: (s: ScreenState) => void;
    onHello: (h: ScreenHello) => void;
    onDisplay: (id: number) => void;
    onQuality: (q: Quality) => void;
    onSize: (width: number, height: number) => void;
    onClipboard: (text: string) => void;
    onClipboardSaved: (ok: boolean) => void;
    onStats: (s: ScreenStats) => void;
};

function fromBase64(b64: string): Uint8Array {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) {
        out[i] = bin.charCodeAt(i);
    }

    return out;
}

export function webCodecsAvailable(): boolean {
    return typeof window !== 'undefined' && 'VideoDecoder' in window && 'EncodedVideoChunk' in window;
}

export class ScreenClient {
    private ws: WebSocket;
    private decoder: VideoDecoder | null = null;
    private config: VideoDecoderConfig | null = null;
    /** After (re)configuring, delta frames are useless until a key frame. */
    private waitingForKey = true;
    /** Frames handed to the decoder, oldest first, to acknowledge as they come out. */
    private decoding: number[] = [];
    private ctx: CanvasRenderingContext2D | null;
    private ended = false;
    private lastError: string | null = null;
    private live = false;
    private frames = 0;
    private bytes = 0;
    private statsTimer: number;
    private width = 0;
    private height = 0;
    /** Leaving the page ends the session (a page kept for Back must not keep it open). */
    private onPageHide = () => this.close();

    constructor(
        sid: string,
        private canvas: HTMLCanvasElement,
        private h: Handlers,
    ) {
        this.ctx = canvas.getContext('2d', { alpha: false, desynchronized: true });
        const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
        this.ws = new WebSocket(`${scheme}//${location.host}/api/screen/ws?sid=${encodeURIComponent(sid)}`);
        this.ws.binaryType = 'arraybuffer';
        this.ws.onmessage = (e) => this.message(e.data);
        this.ws.onclose = () => this.finish(null);
        this.ws.onerror = () => {
            this.lastError ??= 'The connection to the server dropped.';
        };
        this.statsTimer = window.setInterval(() => {
            this.h.onStats({ fps: this.frames, kbps: Math.round((this.bytes * 8) / 1000), width: this.width, height: this.height });
            this.frames = 0;
            this.bytes = 0;
        }, 1000);
        window.addEventListener('pagehide', this.onPageHide);
        h.onState({ state: 'connecting' });
    }

    private finish(message: string | null) {
        if (this.ended) {
            return;
        }
        this.ended = true;
        window.clearInterval(this.statsTimer);
        window.removeEventListener('pagehide', this.onPageHide);
        try {
            this.decoder?.close();
        } catch {
            // Already closed.
        }
        this.decoder = null;
        this.h.onState({ state: 'ended', message: message ?? this.lastError });
    }

    close() {
        this.send({ type: 'release' });
        this.ws.close();
        this.finish(null);
    }

    send(msg: Record<string, unknown>) {
        if (this.ws.readyState === WebSocket.OPEN) {
            this.ws.send(JSON.stringify(msg));
        }
    }

    private message(data: unknown) {
        if (typeof data === 'string') {
            let m: Record<string, unknown>;
            try {
                m = JSON.parse(data);
            } catch {
                return;
            }
            this.control(m);
        } else if (data instanceof ArrayBuffer) {
            this.frame(data);
        }
    }

    private control(m: Record<string, unknown>) {
        switch (m.type) {
            case 'hello':
                this.h.onHello(m as unknown as ScreenHello);
                break;
            case 'status':
                if (m.state === 'confirming') {
                    this.h.onState({ state: 'confirming' });
                }
                break;
            case 'config':
                this.configure({
                    codec: String(m.codec),
                    description: fromBase64(String(m.description)),
                    codedWidth: Number(m.width),
                    codedHeight: Number(m.height),
                    optimizeForLatency: true,
                });
                break;
            case 'display':
                this.h.onDisplay(Number(m.display));
                break;
            case 'quality':
                this.h.onQuality(m.quality as Quality);
                break;
            case 'clipboard':
                this.h.onClipboard(String(m.text ?? ''));
                break;
            case 'clipboard.saved':
                this.h.onClipboardSaved(!!m.ok);
                break;
            case 'error':
                this.lastError = String(m.message ?? 'The session ended.');
                break;
        }
    }

    /** Acknowledge frames that won't come out of the decoder (reset or skipped). */
    private releaseDecoding() {
        for (const seq of this.decoding) {
            this.send({ type: 'ack', seq });
        }
        this.decoding = [];
    }

    private newDecoder(): VideoDecoder {
        return new VideoDecoder({
            output: (frame) => this.draw(frame),
            error: (e) => {
                console.warn('[screen] decoder error', e);
                this.decoder = null;
                this.releaseDecoding();
                if (this.config && !this.ended) {
                    this.configure(this.config);
                    this.send({ type: 'keyframe' });
                }
            },
        });
    }

    private configure(config: VideoDecoderConfig) {
        this.config = config;
        this.releaseDecoding();
        try {
            if (!this.decoder || this.decoder.state === 'closed') {
                this.decoder = this.newDecoder();
            } else {
                this.decoder.reset();
            }
            this.decoder.configure(config);
        } catch (e) {
            console.warn('[screen] configure failed', e);
            this.lastError = 'This browser can’t decode the screen video.';
            this.ws.close();
            return;
        }
        this.waitingForKey = true;
    }

    private frame(buf: ArrayBuffer) {
        if (buf.byteLength < 14) {
            return;
        }
        const view = new DataView(buf);
        if (view.getUint8(0) !== 1) {
            return;
        }
        const key = (view.getUint8(1) & 1) === 1;
        const seq = view.getUint32(2);
        const ts = Number(view.getBigUint64(6));
        this.bytes += buf.byteLength;
        if (!this.decoder || this.decoder.state !== 'configured' || (this.waitingForKey && !key)) {
            this.send({ type: 'ack', seq });
            if (this.decoder && this.waitingForKey && !key) {
                this.send({ type: 'keyframe' });
            }
            return;
        }
        this.waitingForKey = false;
        this.decoding.push(seq);
        try {
            this.decoder.decode(new EncodedVideoChunk({ type: key ? 'key' : 'delta', timestamp: ts, data: new Uint8Array(buf, 14) }));
        } catch (e) {
            console.warn('[screen] decode failed', e);
            this.releaseDecoding();
            this.waitingForKey = true;
            this.send({ type: 'keyframe' });
        }
    }

    private draw(frame: VideoFrame) {
        const w = frame.displayWidth;
        const h = frame.displayHeight;
        if (this.canvas.width !== w || this.canvas.height !== h) {
            this.canvas.width = w;
            this.canvas.height = h;
            this.width = w;
            this.height = h;
            this.h.onSize(w, h);
        }
        this.ctx?.drawImage(frame, 0, 0);
        frame.close();
        this.frames++;
        const seq = this.decoding.shift();
        if (seq !== undefined) {
            this.send({ type: 'ack', seq });
        }
        if (!this.live) {
            this.live = true;
            this.h.onState({ state: 'live' });
        }
    }

    // ---- input ---------------------------------------------------------------------------

    pointer(x: number, y: number, action: 'move' | 'down' | 'up', button = 0, clicks = 0) {
        this.send({ type: 'pointer', x, y, action, button, clicks });
    }

    wheel(dx: number, dy: number) {
        this.send({ type: 'wheel', dx, dy });
    }

    key(code: string, down: boolean, repeat = false) {
        this.send({ type: 'key', code, down, repeat });
    }

    /** A key pressed and let go, with modifiers held around it. */
    combo(modifiers: string[], code: string) {
        for (const m of modifiers) this.key(m, true);
        this.key(code, true);
        this.key(code, false);
        for (const m of [...modifiers].reverse()) this.key(m, false);
    }

    text(text: string) {
        this.send({ type: 'text', text });
    }

    release() {
        this.send({ type: 'release' });
    }

    display(id: number) {
        this.send({ type: 'display', id });
    }

    quality(quality: Quality) {
        this.send({ type: 'quality', quality });
    }

    setClipboard(text: string) {
        this.send({ type: 'clipboard.set', text });
    }

    getClipboard() {
        this.send({ type: 'clipboard.get' });
    }
}
