// Settings › Remote access: link this computer to an ExampleLab account so its sessions
// can be watched and answered from a browser or phone. The background service holds the
// link (it keeps working with the window closed); this only shows and changes it.

import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { api, type RemoteStatus } from "./api";

/** The ExampleLab server new links go to. Editable in the form. */
export const DEFAULT_REMOTE_SERVER = "https://examplelab.com";

const STATE_TEXT: Record<RemoteStatus["state"], { label: string; dot: string }> = {
  unpaired: { label: "Not linked", dot: "" },
  connecting: { label: "Connecting…", dot: "" },
  connected: { label: "Connected", dot: " ok" },
  no_plan: { label: "No remote plan", dot: " bad" },
  error: { label: "Can't connect", dot: " bad" },
};

export function RemoteSection({
  Group,
  Row,
}: {
  Group: (p: { title: string; note?: ReactNode; children: ReactNode }) => ReactNode;
  Row: (p: { title: ReactNode; desc?: ReactNode; labelId?: string; children?: ReactNode }) => ReactNode;
}) {
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    const load = () =>
      api.remoteStatus().then(
        (s) => {
          if (!live) return;
          setStatus(s);
          setLoadError(null);
        },
        (e) => live && setLoadError(String(e)),
      );
    void load();
    const timer = window.setInterval(load, 2000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, []);

  if (!status) {
    return (
      <Group title="Remote access">
        <Row
          title={loadError ? "Not available" : "Loading…"}
          desc={loadError ?? "Asking the background service."}
        />
      </Group>
    );
  }

  return status.state === "unpaired" ? (
    <LinkForm status={status} onLinked={setStatus} Group={Group} />
  ) : (
    <Linked status={status} onUnlinked={setStatus} Group={Group} Row={Row} />
  );
}

function LinkForm({
  status,
  onLinked,
  Group,
}: {
  status: RemoteStatus;
  onLinked(s: RemoteStatus): void;
  Group: (p: { title: string; note?: ReactNode; children: ReactNode }) => ReactNode;
}) {
  const [server, setServer] = useState(DEFAULT_REMOTE_SERVER);
  const [code, setCode] = useState("");
  const [name, setName] = useState(status.defaultMachineName);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onLinked(await api.remotePair(server, code, name));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  const dashboard = `${server.replace(/\/+$/, "") || DEFAULT_REMOTE_SERVER}/dashboard`;

  return (
    <Group
      title="Link this computer"
      note={
        <>
          Termlab sends session status, approval prompts and a few preview lines. Terminal output and
          your keystrokes only travel while you have that terminal open in a browser, and aren't stored.
        </>
      }
    >
      <form className="settings-block" onSubmit={submit}>
        {status.error && <p className="settings-warning">{status.error}</p>}
        <p className="muted small" style={{ margin: 0, lineHeight: 1.5 }}>
          With a Remote plan you can watch and answer this computer's sessions from a browser or your
          phone. In your ExampleLab dashboard, click <strong>Pair a machine</strong> and enter the code here.{" "}
          <button type="button" className="link-btn small" onClick={() => void api.openExternal(dashboard).catch(() => {})}>
            Open the dashboard
          </button>
        </p>
        <label className="field">
          <span className="label">Pairing code</span>
          <input
            className="input mono"
            value={code}
            onChange={(e) => setCode(e.target.value.toUpperCase())}
            placeholder="K7QX-2MPD"
            autoComplete="off"
            spellCheck={false}
            required
          />
        </label>
        <div className="two-col">
          <label className="field">
            <span className="label">Name for this computer</span>
            <input className="input" value={name} onChange={(e) => setName(e.target.value)} required />
          </label>
          <label className="field">
            <span className="label">ExampleLab server</span>
            <input
              className="input mono"
              value={server}
              onChange={(e) => setServer(e.target.value)}
              spellCheck={false}
              required
            />
          </label>
        </div>
        {error && (
          <p className="error-text small" role="alert" style={{ margin: 0 }}>
            {error}
          </p>
        )}
        <div>
          <button className="btn primary" type="submit" disabled={busy || !code.trim()}>
            {busy ? "Linking…" : "Link this computer"}
          </button>
        </div>
      </form>
    </Group>
  );
}

function Linked({
  status,
  onUnlinked,
  Group,
  Row,
}: {
  status: RemoteStatus;
  onUnlinked(s: RemoteStatus): void;
  Group: (p: { title: string; note?: ReactNode; children: ReactNode }) => ReactNode;
  Row: (p: { title: ReactNode; desc?: ReactNode; labelId?: string; children?: ReactNode }) => ReactNode;
}) {
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const state = STATE_TEXT[status.state];
  const server = status.serverUrl ?? DEFAULT_REMOTE_SERVER;

  async function unlink() {
    setBusy(true);
    setError(null);
    try {
      onUnlinked(await api.remoteUnpair());
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  const desc =
    status.state === "connected"
      ? `${status.sessions} ${status.sessions === 1 ? "session is" : "sessions are"} visible remotely` +
        (status.viewers > 0 ? ` · ${status.viewers} watching a terminal now` : "") +
        ". This keeps working with Termlab closed."
      : (status.error ?? "Termlab retries by itself.");

  return (
    <>
      <Group title="Connection">
        <Row
          title={
            <span className="settings-status">
              <span className={`dot${state.dot}`} aria-hidden="true" />
              {state.label}
            </span>
          }
          desc={desc}
        >
          <button className="btn small" onClick={() => void api.openExternal(`${server}/remote`).catch(() => {})}>
            Open Termlab Remote
          </button>
        </Row>
        <Row title="Account" desc={`${status.accountName ?? "—"}${status.plan ? ` · ${status.plan} plan` : ""}`} />
        <Row title="This computer" desc={`${status.machineName ?? "—"} · ${server}`} />
      </Group>
      <Group title="Unlink" note="The computer disappears from your ExampleLab dashboard. Sessions here keep running.">
        <Row
          title={confirm ? "Unlink this computer?" : "Stop sharing this computer"}
          desc={confirm ? "You can link it again later with a new pairing code." : undefined}
        >
          {confirm ? (
            <span style={{ display: "flex", gap: 8 }}>
              <button className="btn small" onClick={() => setConfirm(false)} disabled={busy}>
                Cancel
              </button>
              <button className="btn small danger" onClick={() => void unlink()} disabled={busy}>
                {busy ? "Unlinking…" : "Unlink"}
              </button>
            </span>
          ) : (
            <button className="btn small" onClick={() => setConfirm(true)}>
              Unlink…
            </button>
          )}
        </Row>
        {error && (
          <p className="error-text small" role="alert" style={{ margin: "0 14px 12px" }}>
            {error}
          </p>
        )}
      </Group>
    </>
  );
}
