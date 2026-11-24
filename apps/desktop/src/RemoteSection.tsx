// Settings › Remote access: link this computer to your own Termlab server (termlab-server,
// self-hosted) so its sessions can be watched and answered from a browser or phone. The
// background service holds the link (it keeps working with the window closed); this only
// shows and changes it.

import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { api, type RemoteStatus, type ScreenStatus } from "./api";

/** How to run a Termlab server. */
export const SELF_HOSTING_GUIDE = "https://github.com/pascallieverse/termlab/blob/main/docs/self-hosting.md";

const STATE_TEXT: Record<RemoteStatus["state"], { label: string; dot: string }> = {
  unpaired: { label: "Not linked", dot: "" },
  connecting: { label: "Connecting…", dot: "" },
  connected: { label: "Connected", dot: " ok" },
  no_plan: { label: "Not allowed", dot: " bad" },
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
  const [server, setServer] = useState("");
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


  return (
    <Group
      title="Link this computer"
      note={
        <>
          Termlab sends session status, approval prompts and a few preview lines to your server.
          Terminal output and your keystrokes only travel while you have that terminal open in a
          browser, and aren't stored. Use https:// for a server on the internet.
        </>
      }
    >
      <form className="settings-block" onSubmit={submit}>
        {status.error && <p className="settings-warning">{status.error}</p>}
        <p className="muted small" style={{ margin: 0, lineHeight: 1.5 }}>
          Watch and answer this computer's sessions from a browser or your phone, through your own
          Termlab server on a small VPS. Nothing goes through anyone else. On your server, open{" "}
          <strong>Machines › Pair a machine</strong> and enter its address and the code here.{" "}
          <button type="button" className="link-btn small" onClick={() => void api.openExternal(SELF_HOSTING_GUIDE).catch(() => {})}>
            How to set up a server
          </button>
        </p>
        <label className="field">
          <span className="label">Your Termlab server</span>
          <input
            className="input mono"
            value={server}
            onChange={(e) => setServer(e.target.value)}
            placeholder="https://remote.example.com"
            autoComplete="url"
            spellCheck={false}
            required
          />
        </label>
        <div className="two-col">
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
          <label className="field">
            <span className="label">Name for this computer</span>
            <input className="input" value={name} onChange={(e) => setName(e.target.value)} required />
          </label>
        </div>
        {error && (
          <p className="error-text small" role="alert" style={{ margin: 0 }}>
            {error}
          </p>
        )}
        <div>
          <button className="btn primary" type="submit" disabled={busy || !code.trim() || !server.trim()}>
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
  const server = status.serverUrl ?? "";

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
            Open your server
          </button>
        </Row>
        <Row title="Server" desc={`${status.accountName ?? "Termlab server"} · ${server}`} />
        <Row title="This computer" desc={status.machineName ?? "—"} />
      </Group>
      <ScreenSharing Group={Group} Row={Row} />
      <Group title="Unlink" note="The computer disappears from your server's Machines list. Sessions here keep running.">
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

/**
 * Screen sharing: see and control this computer from the server's web page. Off unless
 * switched on here; the server can't switch it on. Each session there also needs a code
 * from the owner's authenticator app.
 */
function ScreenSharing({
  Group,
  Row,
}: {
  Group: (p: { title: string; note?: ReactNode; children: ReactNode }) => ReactNode;
  Row: (p: { title: ReactNode; desc?: ReactNode; labelId?: string; children?: ReactNode }) => ReactNode;
}) {
  const [s, setS] = useState<ScreenStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [approving, setApproving] = useState(false);

  useEffect(() => {
    let live = true;
    const load = () => api.screenStatus().then((x) => live && setS(x), () => {});
    void load();
    const timer = window.setInterval(load, 2000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, []);

  if (!s) return null;

  if (!s.supported) {
    return (
      <Group title="Screen sharing">
        <Row title="Not available" desc="Seeing and controlling this computer's screen from your server works on macOS, Windows and Linux." />
      </Group>
    );
  }

  async function set(enabled: boolean, confirm: boolean) {
    setError(null);
    try {
      setS(await api.screenSet(enabled, confirm));
    } catch (err) {
      setError(String(err));
    }
  }

  /** Wayland: the desktop shows its own dialog now; the approval is kept for later sessions. */
  async function approve() {
    setError(null);
    setApproving(true);
    try {
      await api.screenRequestPermission("screen");
      setS(await api.screenStatus());
    } catch (err) {
      setError(String(err));
    } finally {
      setApproving(false);
    }
  }

  const since = s.active ? new Date(s.active.startedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "";

  return (
    <Group
      title="Screen sharing"
      note={
        <>
          See this computer's screen and use its mouse and keyboard from your server's web page,
          after entering a code from your authenticator app. Only this switch can turn it on. The
          picture and your input go through your server and aren't stored; each session is in its
          audit log.
        </>
      }
    >
      {s.active && (
        <Row
          title={
            <span className="settings-status">
              <span className="dot warn" aria-hidden="true" />
              In use since {since}
            </span>
          }
          desc={s.active.viewer || "Someone signed in to your server"}
        >
          <button className="btn small danger" onClick={() => void api.screenStop().catch((e) => setError(String(e)))}>
            Stop
          </button>
        </Row>
      )}
      <label className="settings-row clickable">
        <span className="settings-row-text">
          <span className="settings-row-title">Allow screen sharing</span>
          <span className="muted small">Your server can show this screen and control it.</span>
        </span>
        <input type="checkbox" role="switch" className="switch" checked={s.enabled} onChange={(e) => void set(e.target.checked, s.confirm)} />
      </label>
      <label className="settings-row clickable">
        <span className="settings-row-text">
          <span className="settings-row-title">Ask on this computer each time</span>
          <span className="muted small">Someone here has to click Allow before a session starts.</span>
        </span>
        <input
          type="checkbox"
          role="switch"
          className="switch"
          checked={s.confirm}
          disabled={!s.enabled}
          onChange={(e) => void set(s.enabled, e.target.checked)}
        />
      </label>
      {s.enabled && s.os === "windows" && (
        <Row
          title="Limits"
          desc="Windows doesn't let a program see the lock screen or a UAC prompt, or control programs running as administrator."
        />
      )}
      {s.enabled && s.os === "linux" && s.wayland && (
        <Row
          title="Desktop approval"
          desc={
            s.screenAllowed
              ? "Approved: sessions start without anyone at this computer."
              : "Your desktop asks once which screen to share and whether to allow control. Until then, someone has to approve each session here."
          }
        >
          {!s.screenAllowed && (
            <button className="btn small primary" disabled={approving} onClick={() => void approve()}>
              {approving ? "Waiting…" : "Approve…"}
            </button>
          )}
        </Row>
      )}
      {s.enabled && s.os === "macos" && (
        <>
          <Row
            title="Screen Recording"
            desc={s.screenAllowed ? "Allowed." : "Needed to show the screen. Allow Termlab in System Settings › Privacy & Security."}
          >
            {!s.screenAllowed && (
              <button className="btn small primary" onClick={() => void api.screenRequestPermission("screen")}>
                Allow…
              </button>
            )}
          </Row>
          <Row
            title="Accessibility"
            desc={s.inputAllowed ? "Allowed: the mouse and keyboard can be used." : "Needed for the mouse and keyboard; without it the screen is view only."}
          >
            {!s.inputAllowed && (
              <button className="btn small primary" onClick={() => void api.screenRequestPermission("input")}>
                Allow…
              </button>
            )}
          </Row>
        </>
      )}
      {error && (
        <p className="error-text small" role="alert" style={{ margin: "0 14px 12px" }}>
          {error}
        </p>
      )}
    </Group>
  );
}
