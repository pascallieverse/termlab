import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useEffect, useRef, useState } from 'react';
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

/**
 * The session's real terminal, streamed from the machine through the server.
 *
 * Opening the view sends `watch`; the machine answers with a snapshot (sized to
 * the pty, so agents' full-screen output isn't garbled), then live output.
 * Keystrokes go back as `input`. Until the machine streams, the last preview
 * lines are shown.
 */
export function LiveTerminal({
    sessionId,
    preview,
    interactive,
}: {
    sessionId: string;
    preview: string[];
    interactive: boolean;
}) {
    const live = useLive();
    const box = useRef<HTMLDivElement>(null);
    const term = useRef<Terminal | null>(null);
    const streaming = useRef(false);
    /** Set once the machine says what size the session's terminal is. */
    const fixedSize = useRef(false);
    const interactiveRef = useRef(interactive);
    const [isLive, setLive] = useState(false);

    interactiveRef.current = interactive;

    // The terminal itself.
    useEffect(() => {
        if (!box.current) {
            return;
        }

        const t = new Terminal({
            convertEol: true,
            cursorBlink: true,
            fontFamily: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
            fontSize: 13,
            lineHeight: 1.35,
            scrollback: 5000,
            theme: {
                background: '#07080B',
                foreground: '#C3C9D2',
                cursor: '#EDEFF3',
                selectionBackground: '#343A46',
                yellow: '#FFC970',
                green: '#8EE6B4',
                red: '#FF9B9B',
                blue: '#8CC4FF',
            },
        });
        const fit = new FitAddon();
        t.loadAddon(fit);
        t.open(box.current);
        fit.fit();
        term.current = t;

        // Fit the box until the machine reports the session's real size.
        const observer = new ResizeObserver(() => {
            if (!fixedSize.current) {
                fit.fit();
            }
        });
        observer.observe(box.current);

        return () => {
            observer.disconnect();
            t.dispose();
            term.current = null;
        };
    }, [sessionId]);

    // Preview lines until real output arrives.
    useEffect(() => {
        const t = term.current;
        if (!t || streaming.current) {
            return;
        }

        t.reset();
        t.write(preview.join('\r\n'));
    }, [preview, sessionId]);

    // The stream.
    useEffect(() => {
        streaming.current = false;
        fixedSize.current = false;
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
                    fixedSize.current = true;
                    t.resize(event.cols, event.rows);
                }
                t.reset();
            } else if (!streaming.current) {
                t.reset();
            }
            streaming.current = true;
            setLive(true);
            t.write(decode(event.data));
        });

        const keys = term.current?.onData((data) => {
            if (interactiveRef.current && streaming.current) {
                live.input(sessionId, encode(data));
            }
        });

        return () => {
            keys?.dispose();
            stop();
        };
        // live.watch and live.input are stable.
    }, [sessionId]);

    return (
        <div className="flex min-h-[260px] flex-1 flex-col overflow-hidden rounded-[10px] border border-line-soft bg-term">
            <div className="flex items-center justify-between border-b border-line-soft px-3 py-1.5 font-mono text-[11px] text-dim">
                <span>
                    {isLive
                        ? 'live'
                        : !interactive
                          ? 'machine offline · last reported lines'
                          : 'waiting for the machine · last reported lines'}
                </span>
                {isLive && interactive && <span>keystrokes go to the session</span>}
            </div>
            <div ref={box} className="min-h-0 flex-1 overflow-auto px-3 py-2" aria-label="Session terminal" role="region" />
        </div>
    );
}
