// Updates of Termlab itself: one shared state for the notice under the header and the
// "Check for updates" button in Settings.

import { useSyncExternalStore } from "react";
import { api, errorText, type UpdateCheck } from "./api";
import { Icon } from "./ui";

interface UpdateState {
  /** The last answer; null before the first check came back. */
  check: UpdateCheck | null;
  /** When that answer came in. */
  checkedAt: number | null;
  checking: boolean;
  installing: boolean;
  error: string | null;
  /** The version whose notice was put off with "Later". */
  dismissed: string | null;
}

let state: UpdateState = { check: null, checkedAt: null, checking: false, installing: false, error: null, dismissed: null };
const listeners = new Set<() => void>();

function set(patch: Partial<UpdateState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

export function useAppUpdate(): UpdateState {
  return useSyncExternalStore(subscribe, () => state);
}

/** A check asked for by hand shows "Checking…" at least this long: the answer usually comes
 *  back at once and is often the same as before, which would look like nothing happened. */
const VISIBLE_CHECK_MS = 700;

export async function checkForUpdate(byHand = false) {
  if (state.checking || state.installing) return;
  set({ checking: true, error: null });
  const shown = new Promise((resolve) => setTimeout(resolve, byHand ? VISIBLE_CHECK_MS : 0));
  try {
    const check = await api.updateCheck();
    await shown;
    set({ check, checkedAt: Date.now(), checking: false });
  } catch (e) {
    await shown;
    set({ checking: false, error: errorText(e) });
  }
}

/** Downloads and installs the new version. Termlab restarts itself when that worked. */
export async function installUpdate() {
  if (state.installing) return;
  set({ installing: true, error: null });
  try {
    await api.updateInstall();
  } catch (e) {
    set({ installing: false, error: errorText(e) });
  }
}

const FIRST_CHECK_MS = 10_000;
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

/** Looks for a new version shortly after launch, then every six hours. */
export function startUpdateChecks(): () => void {
  const first = setTimeout(() => void checkForUpdate(), FIRST_CHECK_MS);
  const repeat = setInterval(() => void checkForUpdate(), CHECK_EVERY_MS);
  return () => {
    clearTimeout(first);
    clearInterval(repeat);
  };
}

// The Windows installer has to stop the background service; elsewhere it keeps running.
const SESSIONS_NOTE = navigator.userAgent.includes("Windows")
  ? "Sessions stop for the update; the ones that can be resumed reopen afterwards."
  : "Your sessions keep running while it restarts.";

/** Shown under the header while a newer version is waiting. */
export function UpdateBanner() {
  const u = useAppUpdate();
  const version = u.check?.version;
  if (!version || u.dismissed === version) return null;
  return (
    <div role="status" className="banner update">
      <span>
        Termlab {version} is available. {SESSIONS_NOTE}
      </span>
      <button className="btn small update-btn" disabled={u.installing} onClick={() => void installUpdate()}>
        <Icon name="refresh" /> {u.installing ? "Updating…" : "Restart to update"}
      </button>
      {!u.installing && (
        <button className="btn small outline update-btn" onClick={() => set({ dismissed: version })}>
          Later
        </button>
      )}
      {u.error && <span>{u.error}</span>}
    </div>
  );
}

function clock(time: number): string {
  return new Date(time).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

/** Settings → About: the version, and a check on request. */
export function UpdateSettings({ version, hideVersion = false }: { version: string | null; hideVersion?: boolean }) {
  const u = useAppUpdate();
  const newer = u.check?.version;
  const status = u.error
    ? u.error
    : u.checking
      ? "Checking…"
      : !u.check
        ? null
        : !u.check.supported
          ? import.meta.env.DEV
            ? "This is a development build, so it doesn't update itself."
            : "This build doesn't update itself. New versions are on the GitHub releases page."
          : newer
            ? `Version ${newer} is available. ${SESSIONS_NOTE}`
            : "You have the latest version.";
  return (
    <>
      {!hideVersion && (
        <p className="muted small">
          Termlab <span className="mono">{version ?? "…"}</span>
        </p>
      )}
      <div className="update-actions">
        <button className="btn" disabled={u.checking || u.installing} onClick={() => void checkForUpdate(true)}>
          <Icon name="refresh" /> {u.checking ? "Checking…" : "Check for updates"}
        </button>
        {newer && (
          <button className="btn primary" disabled={u.installing} onClick={() => void installUpdate()}>
            {u.installing ? "Updating…" : "Restart to update"}
          </button>
        )}
      </div>
      {status && (
        <p className="muted small" role="status">
          {status}
          {!u.checking && !u.error && u.checkedAt && ` Checked at ${clock(u.checkedAt)}.`}
        </p>
      )}
    </>
  );
}
