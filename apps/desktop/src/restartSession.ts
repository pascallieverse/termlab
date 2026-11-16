import type { Session } from "./api";

interface RestartApi {
  session(id: string): Promise<Session>;
  stop(id: string): Promise<unknown>;
  restart(id: string): Promise<Session>;
  /** Asks the agent to restart a background session it runs itself. */
  respawn(id: string): Promise<unknown>;
}

/**
 * Restarts a session on the same conversation, which is also how it gets onto a newly
 * installed version of its agent. Waits for exit/scrollback persistence before starting again.
 */
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
  const ended = async (attempts: number) => {
    for (let attempt = 0; attempt < attempts && session.live; attempt++) {
      await wait();
      session = await api.session(id);
    }
    return !session.live;
  };

  // A background session the agent runs itself: the agent restarts it. Termlab's terminal
  // is only a window onto it, which closes with the restart and is opened again below.
  const attached = session.origin === "attached";
  if (attached && session.live) {
    await api.respawn(id);
    await ended(30);
  }
  if (session.live) {
    await api.stop(id);
    if (!(await ended(100))) throw new Error("The session has not stopped yet. Try restarting again once it stops.");
  }
  if (!attached) return api.restart(id);

  // The restarted background session takes a moment to be listed as running again.
  let failure: unknown;
  for (let attempt = 0; attempt < 15; attempt++) {
    try {
      return await api.restart(id);
    } catch (e) {
      failure = e;
      await wait();
    }
  }
  throw failure;
}
