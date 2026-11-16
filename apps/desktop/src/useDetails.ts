import { useEffect, useState } from "react";
import { api, type SessionDetails, type SessionStatus } from "./api";

/** What each session's details were when last loaded, shown at once when switching back to it. */
const lastKnown = new Map<string, SessionDetails>();

/** Inspector data for a session: refreshed when its status changes and every few seconds. */
export function useDetails(id: string | null, status?: SessionStatus): SessionDetails | null {
  const [loaded, setLoaded] = useState<{ id: string; details: SessionDetails } | null>(null);

  useEffect(() => {
    if (!id) return;
    let cancelled = false;
    const load = () =>
      api.details(id).then(
        (details) => {
          lastKnown.set(id, details);
          if (!cancelled) setLoaded({ id, details });
        },
        () => {},
      );
    void load();
    const active = status === "running" || status === "needs_you" || status === "idle";
    const timer = active ? setInterval(load, 5000) : undefined;
    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, [id, status]);

  if (!id) return null;
  // Never another session's details: right after switching, show this session's last known
  // ones (or nothing) until the fresh ones arrive.
  return loaded?.id === id ? loaded.details : (lastKnown.get(id) ?? null);
}
