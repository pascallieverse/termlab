// Session labels: the chips shown on a session, and the dialog that finds, makes and assigns them.

import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { Label } from "./api";
import { Dialog } from "./Dialog";
import { LABEL_COLOR_NAMES, LABEL_NAME_MAX, labelHex, nextLabelColor } from "./sessionLabels";
import { useStore } from "./store";
import { Icon } from "./ui";

function labelStyle(color: string): Record<string, string> {
  return { "--label-color": labelHex(color) };
}

/** Names compare as the label file stores them: spaces collapsed, capitals ignored. */
function normalName(name: string): string {
  return name.trim().replace(/\s+/g, " ");
}

export function LabelChip({ label }: { label: Pick<Label, "name" | "color"> }) {
  return (
    <span className="label-chip" style={labelStyle(label.color)}>
      <span className="label-chip-dot" aria-hidden="true" />
      <span className="label-chip-name">{label.name}</span>
    </span>
  );
}

/**
 * A session's labels as chips, or as small coloured bars that name themselves on hover where
 * there's no room for words. Nothing when it has none.
 */
export function SessionLabels({ sessionId, dots = false }: { sessionId: string; dots?: boolean }) {
  const labels = useStore().labelsOf(sessionId);
  if (labels.length === 0) return null;
  if (dots) {
    return (
      <span className="label-dots" role="img" aria-label={`Labels: ${labels.map((l) => l.name).join(", ")}`}>
        {labels.map((l) => (
          <span key={l.id} className="label-dot" style={labelStyle(l.color)} title={l.name} />
        ))}
      </span>
    );
  }
  return (
    <span className="label-chips">
      {labels.map((l) => (
        <LabelChip key={l.id} label={l} />
      ))}
    </span>
  );
}

/** Opens the labels dialog for a session: its chips, or an invitation to add the first one. */
export function LabelsButton({ sessionId }: { sessionId: string }) {
  const store = useStore();
  const labels = store.labelsOf(sessionId);
  return (
    <button
      className={`labels-btn${labels.length === 0 ? " empty" : ""}`}
      title="Edit this session's labels"
      onClick={() => store.setLabelsFor(sessionId)}
    >
      {labels.length === 0 ? (
        <>
          <Icon name="tag" size={12} /> Add label
        </>
      ) : (
        <>
          <span className="sr">Labels: </span>
          <SessionLabels sessionId={sessionId} />
        </>
      )}
    </button>
  );
}

/** A row of colour choices. A mouse pick leaves the keyboard where it was (`keep`). */
function Swatches({
  value,
  onChange,
  keep,
  small = false,
  name,
}: {
  value: string;
  onChange(color: string): void;
  keep(): void;
  small?: boolean;
  name: string;
}) {
  return (
    <div role="radiogroup" aria-label="Colour" className={`swatches${small ? " small" : ""}`}>
      {LABEL_COLOR_NAMES.map((c) => (
        <label
          key={c}
          className="swatch"
          title={c[0].toUpperCase() + c.slice(1)}
          style={labelStyle(c)}
          onMouseDown={(e) => {
            e.preventDefault();
            onChange(c);
            keep();
          }}
        >
          <input type="radio" name={name} className="sr" value={c} checked={value === c} onChange={() => onChange(c)} />
          <span aria-hidden="true" />
          <span className="sr">{c}</span>
        </label>
      ))}
    </div>
  );
}

/** Renaming, recolouring or deleting one label, in place of its row. */
function LabelEditor({ label, onDone }: { label: Label; onDone(): void }) {
  const store = useStore();
  const [name, setName] = useState(label.name);
  const [color, setColor] = useState(label.color);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const used = store.sessions.filter((s) => store.labelsOf(s.id).some((l) => l.id === label.id)).length;
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);

  async function save() {
    if (!normalName(name) || busy) return;
    setBusy(true);
    const saved = await store.saveLabel(label.id, name, color);
    setBusy(false);
    if (saved) onDone();
  }

  function remove() {
    const go = () => {
      onDone();
      void store.deleteLabel(label.id);
    };
    if (used === 0) return go();
    store.confirm({
      title: `Delete the label “${label.name}”?`,
      body: `It is on ${used} session${used === 1 ? "" : "s"} and will be taken off ${used === 1 ? "it" : "them"}. The sessions themselves are not touched.`,
      action: "Delete label",
      onConfirm: go,
    });
  }

  return (
    <form
      className="label-editor"
      aria-label={`Edit the label ${label.name}`}
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          // Back to the list, not out of the dialog.
          e.preventDefault();
          e.stopPropagation();
          onDone();
        }
      }}
    >
      <div className="label-editor-row">
        <span className="label-editor-dot" style={labelStyle(color)} aria-hidden="true" />
        <input
          ref={input}
          className="input"
          aria-label="Name"
          value={name}
          maxLength={LABEL_NAME_MAX}
          autoComplete="off"
          onChange={(e) => setName(e.target.value)}
        />
      </div>
      <Swatches name={`lb-edit-${label.id}`} value={color} onChange={setColor} keep={() => input.current?.focus()} />
      <div className="label-editor-actions">
        <button type="submit" className="btn small primary" disabled={!normalName(name) || busy}>
          Save
        </button>
        <button type="button" className="btn small" onClick={onDone}>
          Cancel
        </button>
        <div className="spacer" />
        <button type="button" className="btn small ghost-danger" onClick={remove}>
          <Icon name="trash" size={13} /> Delete
        </button>
      </div>
    </form>
  );
}

/**
 * The labels of one session. A single field finds a label to tick, or makes a new one from
 * what's typed; the arrow keys and Enter work the list without the mouse.
 */
export function LabelsDialog() {
  const store = useStore();
  const s = store.sessions.find((x) => x.id === store.labelsFor);
  const { labels } = store;
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [newColor, setNewColor] = useState(() => nextLabelColor(labels));
  const [editingId, setEditingId] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const field = useRef<HTMLInputElement>(null);
  const close = () => store.setLabelsFor(null);
  if (!s) return null;

  const sessionId = s.id;
  const mine = store.labelsOf(sessionId).map((l) => l.id);
  const typed = normalName(query);
  const shown = typed ? labels.filter((l) => l.name.toLowerCase().includes(typed.toLowerCase())) : labels;
  const exact = labels.some((l) => l.name.toLowerCase() === typed.toLowerCase());
  const canCreate = typed !== "" && !exact;
  // The list's rows, then the "create" row: what the arrow keys move through.
  const count = shown.length + (canCreate ? 1 : 0);
  const current = Math.min(active, Math.max(0, count - 1));
  const usedBy = (id: number) => store.sessions.filter((x) => store.labelsOf(x.id).some((l) => l.id === id)).length;
  const focusField = () => field.current?.focus();

  function toggle(id: number) {
    void store.setSessionLabels(sessionId, mine.includes(id) ? mine.filter((x) => x !== id) : [...mine, id]);
  }

  async function create() {
    if (!canCreate || busy) return;
    setBusy(true);
    try {
      const saved = await store.saveLabel(null, typed, newColor);
      if (!saved) return;
      // A label made here is meant for this session.
      await store.setSessionLabels(sessionId, [...mine, saved.id]);
      setQuery("");
      setActive(0);
      setNewColor(nextLabelColor([...labels, saved]));
    } finally {
      setBusy(false);
      focusField();
    }
  }

  function choose(i: number) {
    if (i < shown.length) toggle(shown[i].id);
    else void create();
  }

  function onFieldKey(e: ReactKeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (count > 0) setActive((current + (e.key === "ArrowDown" ? 1 : count - 1)) % count);
    } else if (e.key === "Enter") {
      e.preventDefault();
      // What's typed names a label exactly: Enter ticks that one rather than the highlighted.
      const match = shown.findIndex((l) => l.name.toLowerCase() === typed.toLowerCase());
      if (count > 0) choose(match >= 0 ? match : current);
    } else if (e.key === "Escape" && query) {
      // Clears the search first; a second Escape closes the dialog.
      e.stopPropagation();
      setQuery("");
      setActive(0);
    }
  }

  return (
    <Dialog onClose={close} width={460} className="labels-dialog" labelledBy="lb-title">
      <div className="dialog-head">
        <div className="dialog-title">
          <h1 id="lb-title">Labels</h1>
          <span className="muted small label-dialog-session">{s.title}</span>
        </div>
        <div className="spacer" />
        <button className="icon-btn" aria-label="Close" onClick={close}>
          <Icon name="close" />
        </button>
      </div>

      <div className="label-search">
        <Icon name={labels.length === 0 ? "tag" : "search"} size={15} />
        <input
          ref={field}
          data-autofocus
          role="combobox"
          aria-label={labels.length === 0 ? "New label name" : "Find or create a label"}
          aria-expanded="true"
          aria-controls="lb-list"
          aria-activedescendant={count > 0 ? `lb-opt-${current}` : undefined}
          value={query}
          maxLength={LABEL_NAME_MAX}
          autoComplete="off"
          spellCheck={false}
          placeholder={labels.length === 0 ? "Name your first label, e.g. Bug" : "Find or create a label…"}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={onFieldKey}
        />
      </div>

      <div className="label-picker">
        <div role="listbox" id="lb-list" aria-label="Labels" aria-multiselectable="true" className="label-list">
          {shown.map((l, i) =>
            l.id === editingId ? (
              <LabelEditor
                key={l.id}
                label={l}
                onDone={() => {
                  setEditingId(null);
                  focusField();
                }}
              />
            ) : (
              <div
                key={l.id}
                id={`lb-opt-${i}`}
                role="option"
                aria-selected={mine.includes(l.id)}
                className={`label-option${i === current ? " is-active" : ""}`}
                onMouseMove={() => setActive(i)}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => {
                  toggle(l.id);
                  focusField();
                }}
              >
                <span className={`label-check${mine.includes(l.id) ? " on" : ""}`} aria-hidden="true">
                  {mine.includes(l.id) && (
                    <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3.5" strokeLinecap="round" strokeLinejoin="round">
                      <path d="m5 12.5 4.5 4.5L19 7.5" />
                    </svg>
                  )}
                </span>
                <LabelChip label={l} />
                <span className="spacer" />
                <span className="label-used" title={`On ${usedBy(l.id)} session${usedBy(l.id) === 1 ? "" : "s"}`}>
                  {usedBy(l.id) || ""}
                </span>
                <button
                  type="button"
                  className="icon-btn small label-edit"
                  aria-label={`Edit the label ${l.name}`}
                  title="Rename, recolour or delete"
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditingId(l.id);
                  }}
                >
                  <Icon name="pencil" size={13} />
                </button>
              </div>
            ),
          )}

          {canCreate && (
            <div
              id={`lb-opt-${shown.length}`}
              role="option"
              aria-selected={false}
              className={`label-create${current === shown.length ? " is-active" : ""}`}
              onMouseMove={() => setActive(shown.length)}
            >
              <button type="button" className="label-create-btn" disabled={busy} onMouseDown={(e) => e.preventDefault()} onClick={() => void create()}>
                <Icon name="plus" size={13} />
                <span>Create</span>
                <LabelChip label={{ name: typed, color: newColor }} />
              </button>
              <Swatches name="lb-new" small value={newColor} onChange={setNewColor} keep={focusField} />
            </div>
          )}

          {labels.length > 0 && shown.length === 0 && !canCreate && <div className="label-empty">No labels match.</div>}
          {labels.length === 0 && !canCreate && (
            <div className="label-empty">
              Labels mark sessions with a coloured name, like “Bug”, “Client work” or “Waiting on review”. Type one
              above and press Enter.
            </div>
          )}
        </div>
      </div>

      <div className="dialog-foot labels-foot">
        <span className="faint small labels-hint">{labels.length > 0 ? "↑↓ move · ↵ add or remove" : "↵ create"}</span>
        <div className="spacer" />
        <button className="btn" onClick={close}>
          Done
        </button>
      </div>
    </Dialog>
  );
}
