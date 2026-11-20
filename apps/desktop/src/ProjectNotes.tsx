import { useEffect, useRef, useState } from "react";
import { api, errorText, type Folder } from "./api";
import { useStore } from "./store";

/** Typing pauses this long before the notes are saved. */
const SAVE_DELAY = 600;
/** How often saves made elsewhere (in the browser, through a Termlab server) are picked up. */
const POLL_MS = 3000;

/**
 * A project's notes, in the right column. Shared by every session in the project (kept by
 * the app per folder path, see project_notes.rs) and saved as you type.
 */
export function ProjectNotes({ folder }: { folder: Folder }) {
  const store = useStore();
  const [text, setText] = useState("");
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  // What the service has, the latest text, and the pending save: read by the timer and
  // by the save on unmount, which outlive the render that scheduled them.
  const saved = useRef("");
  const latest = useRef("");
  const timer = useRef<number | undefined>(undefined);
  // Saves run one after another, so an older text never lands after a newer one.
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const mounted = useRef(true);
  const toast = store.toast;

  useEffect(() => {
    mounted.current = true;
    api.readProjectNotes(folder.path).then(
      (n) => {
        if (!mounted.current) return;
        saved.current = latest.current = n.text;
        setText(n.text);
        setLoading(false);
      },
      (e: unknown) => {
        if (!mounted.current) return;
        setLoadError(errorText(e));
      },
    );
    // Edits made in the browser land in the same file: take them, unless there's unsaved
    // typing here (that wins, as the next save).
    const poll = window.setInterval(() => {
      if (latest.current !== saved.current) return;
      api.readProjectNotes(folder.path).then(
        (n) => {
          if (!mounted.current || latest.current !== saved.current || n.text === saved.current) return;
          saved.current = latest.current = n.text;
          setText(n.text);
        },
        () => {},
      );
    }, POLL_MS);
    return () => {
      window.clearInterval(poll);
      mounted.current = false;
      // Switching project or closing the panel keeps what was typed.
      window.clearTimeout(timer.current);
      flush();
    };
    // `flush` reads refs only; this runs once per project (the component is keyed by it).
  }, [folder.path]);

  function flush() {
    window.clearTimeout(timer.current);
    const value = latest.current;
    if (value === saved.current) return;
    queue.current = queue.current.then(() =>
      api.saveProjectNotes(folder.path, value).then(
        () => {
          saved.current = value;
        },
        (e: unknown) => toast(`Couldn't save the notes for ${folder.name}: ${errorText(e)}`),
      ),
    );
  }

  function change(value: string) {
    latest.current = value;
    setText(value);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(flush, SAVE_DELAY);
  }

  return (
    <section className="notes" aria-label={`Notes for ${folder.name}`}>
      {loadError ? (
        <p className="error-text small">{loadError}</p>
      ) : (
        <textarea
          className="textarea notes-text"
          aria-label={`Notes for ${folder.name}`}
          placeholder="Write anything worth remembering about this project: commands, links, decisions, to-dos…"
          value={text}
          disabled={loading}
          onChange={(e) => change(e.target.value)}
          onBlur={flush}
          onKeyDown={(e) => {
            // Esc leaves the notes (back to whatever had focus, usually the terminal).
            if (e.key === "Escape") e.currentTarget.blur();
          }}
        />
      )}
    </section>
  );
}
