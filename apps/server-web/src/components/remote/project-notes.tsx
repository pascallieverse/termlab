import { useEffect, useRef, useState } from 'react';
import { useLive } from '@/live/LiveProvider';
import type { RemoteSession } from '@/types';

/** Typing pauses this long before the notes are saved. */
const SAVE_DELAY = 700;

/**
 * A project's notes, the same ones as in Termlab's right column: kept on the machine, shared
 * by every session in the project, and saved as you type. Edits made in Termlab show up here,
 * unless you have unsaved typing of your own.
 */
export function ProjectNotes({ session, online }: { session: RemoteSession; online: boolean }) {
    const live = useLive();
    const machine = live.machines.find((m) => m.id === session.machine_id);
    const path = session.directory ?? null;
    const stored = path ? (live.notes.find((n) => n.machine_id === session.machine_id && n.path === path)?.text ?? '') : '';
    const supported = !!path && !!machine?.notes_sync;
    const editable = supported && online;

    const [text, setText] = useState(stored);
    // What the machine has, and what was typed: read by the timer and by the save on unmount.
    const saved = useRef(stored);
    const latest = useRef(stored);
    const timer = useRef<number | undefined>(undefined);
    const queue = useRef<Promise<unknown>>(Promise.resolve());

    // Changes from the machine (saved in Termlab, or our own save coming back).
    useEffect(() => {
        if (latest.current === saved.current) {
            latest.current = stored;
            setText(stored);
        }
        saved.current = stored;
    }, [stored]);

    useEffect(
        () => () => {
            window.clearTimeout(timer.current);
            flush();
        },
        // One project per mount (the component is keyed by it); flush reads refs only.
        // eslint-disable-next-line react-hooks/exhaustive-deps
        [],
    );

    function flush() {
        window.clearTimeout(timer.current);
        const value = latest.current;
        if (!path || value === saved.current) return;
        const machineId = session.machine_id;
        queue.current = queue.current.then(async () => {
            if (await live.saveNotes(machineId, path, value)) {
                saved.current = value;
            }
        });
    }

    function change(value: string) {
        latest.current = value;
        setText(value);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(flush, SAVE_DELAY);
    }

    return (
        <section className="notes" aria-label={`Notes for ${session.project}`}>
            {!supported && (
                <p className="m-0 text-[13px] leading-relaxed text-dim">
                    Project notes show up here once {session.machine} runs Termlab 0.2.9 or newer.
                </p>
            )}
            {supported && (
                <>
                    <textarea
                        className="notes-text"
                        aria-label={`Notes for ${session.project}`}
                        placeholder="Write anything worth remembering about this project: commands, links, decisions, to-dos…"
                        value={text}
                        readOnly={!editable}
                        onChange={(e) => change(e.target.value)}
                        onBlur={flush}
                        onKeyDown={(e) => {
                            if (e.key === 'Escape') e.currentTarget.blur();
                        }}
                    />
                    <p className="m-0 text-xs text-faint">
                        {editable
                            ? 'Shared by every session in this project, and synced with Termlab.'
                            : `${session.machine} is offline: these are the last notes it sent.`}
                    </p>
                </>
            )}
        </section>
    );
}
