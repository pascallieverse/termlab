import type { Session } from "./api";

export type SessionFilter = "all" | "attention" | "running" | "on_hold";

/** Hold is independent of process status, including updates from a running agent. */
export function matchesSessionFilter(s: Pick<Session, "status" | "onHold">, filter: SessionFilter): boolean {
  if (filter === "on_hold") return !!s.onHold;
  if (s.onHold) return false;
  if (filter === "attention") return s.status === "needs_you" || s.status === "error";
  if (filter === "running") return s.status === "running";
  return true;
}
