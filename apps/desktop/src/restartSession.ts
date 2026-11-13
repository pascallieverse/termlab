import type { Session } from "./api";

interface RestartApi {
  session(id: string): Promise<Session>;
  stop(id: string): Promise<unknown>;
  restart(id: string): Promise<Session>;
}

/** Wait for exit/scrollback persistence before starting the same conversation again. */
export async function restartSession(
  id: string,
  api: RestartApi,
  canResume: boolean,
  wait: () => Promise<void> = () => new Promise((resolve) => setTimeout(resolve, 100)),
): Promise<Session> {
  let session = await api.session(id);
  if (!canResume || !session.agentSessionId) {
    throw new Error("This session cannot be resumed yet, so it has not been stopped.");
  }
  if (session.origin === "attached") {
    throw new Error("This agent runs outside Termlab. Restart it in its original terminal.");
  }
  if (session.live) {
    await api.stop(id);
    for (let attempt = 0; attempt < 100; attempt++) {
      session = await api.session(id);
      if (!session.live) break;
      await wait();
    }
    if (session.live) throw new Error("The session has not stopped yet. Try restarting again once it stops.");
  }
  return api.restart(id);
}
