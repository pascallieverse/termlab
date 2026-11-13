import { useEffect, useState } from "react";
import { api, errorText, type Folder } from "./api";
import { Dialog } from "./Dialog";
import { EnvEditor } from "./EnvEditor";
import { useStore } from "./store";

export function EnvDialog({ folder }: { folder: Folder }) {
  const store = useStore();
  const [original, setOriginal] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [discarding, setDiscarding] = useState(false);
  const dirty = loaded && content !== (original ?? "");

  useEffect(() => {
    let active = true;
    api.readProjectEnv(folder.path).then((value) => {
      if (!active) return;
      setOriginal(value);
      setContent(value ?? "");
      setLoaded(true);
    }).catch((e) => {
      if (active) setError(errorText(e));
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => { active = false; };
  }, [folder.path]);

  function close() {
    if (saving) return;
    if (dirty) setDiscarding(true);
    else store.setEnvFolder(null);
  }

  async function save() {
    if (!loaded || saving) return;
    setSaving(true);
    setError("");
    try {
      // Textareas normalize newlines. Keep the file's existing newline convention.
      const text = original?.includes("\r\n") ? content.replace(/\r?\n/g, "\r\n") : content;
      await api.saveProjectEnv(folder.path, text, original);
      store.toast(`Saved .env in ${folder.name}`, "info");
      store.setEnvFolder(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog onClose={close} width={800} labelledBy="env-title">
      <div className="dialog-head"><h1 id="env-title">Edit .env — {folder.name}</h1></div>
      <div className="dialog-body">
        <p className="muted small mono" style={{ overflowWrap: "anywhere" }}>{folder.path}/.env</p>
        {loading && <p role="status">Loading…</p>}
        {loaded && <>
          {original === null && <p className="muted small">No .env file yet. Save to create it.</p>}
          <label className="sr" htmlFor="env-content">.env contents</label>
          <EnvEditor
            value={content}
            onChange={(value) => { setContent(value); setDiscarding(false); }}
            disabled={saving}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "s") {
                e.preventDefault();
                void save();
              }
            }}
          />
        </>}
        {error && <p role="alert" className="error-text">{error}</p>}
        {discarding && <p role="alert">Discard your unsaved changes?</p>}
      </div>
      <div className="dialog-foot">
        <div className="spacer" />
        {discarding ? <>
          <button className="btn tall" onClick={() => setDiscarding(false)}>Keep editing</button>
          <button className="btn danger tall" onClick={() => store.setEnvFolder(null)}>Discard changes</button>
        </> : <>
          <button className="btn tall" disabled={saving} onClick={close}>Cancel</button>
          <button className="btn primary tall" disabled={!loaded || saving || (!dirty && original !== null)} onClick={() => void save()}>
            {saving ? "Saving…" : "Save"}
          </button>
        </>}
      </div>
    </Dialog>
  );
}
