import { useEffect, useState } from "react";
import { api, type SessionDetails, type SessionStatus } from "./api";

/** Inspector data for a session: refreshed when its status changes and every few seconds. */
export function useDetails(id: string | null, status?: SessionStatus): SessionDetails | null {
  const [details, setDetails] = useState<SessionDetails | null>(null);

  useEffect(() => {
    setDetails(null);
  }, [id]);

  useEffect(() => {
    if (!id) return;
    let cancelled = false;
    const load = () =>
      api.details(id).then(
        (d) => !cancelled && setDetails(d),
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

  return details;
}
