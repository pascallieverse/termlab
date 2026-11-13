import assert from "node:assert/strict";
import test from "node:test";
import { restartSession } from "../src/restartSession.ts";

const session = { id: "session", live: true, agentSessionId: "conversation", origin: "termlab" };

test("waits for exit before resuming the same session", async () => {
  const events = [];
  let reads = 0;
  const resumed = { ...session, title: "SEO optimizer" };
  const api = {
    session: async (id) => {
      assert.equal(id, session.id);
      events.push("read");
      return { ...session, live: ++reads < 3 };
    },
    stop: async (id) => { assert.equal(id, session.id); events.push("stop"); },
    restart: async (id) => {
      assert.equal(id, session.id);
      assert.equal(reads, 3);
      events.push("restart");
      return resumed;
    },
  };
  assert.equal(await restartSession(session.id, api, true, async () => { events.push("wait"); }), resumed);
  assert.deepEqual(events, ["read", "stop", "read", "wait", "read", "restart"]);
});

test("refuses unsafe restarts before stopping", async () => {
  for (const [data, canResume] of [
    [{ ...session, agentSessionId: null }, true],
    [{ ...session, origin: "attached" }, true],
    [session, false],
  ]) {
    await assert.rejects(restartSession(session.id, {
      session: async () => data,
      stop: async () => assert.fail("must not stop"),
      restart: async () => assert.fail("must not restart"),
    }, canResume));
  }
});

test("does not start a second process after a stop timeout", async () => {
  let waits = 0;
  await assert.rejects(restartSession(session.id, {
    session: async () => session,
    stop: async () => {},
    restart: async () => assert.fail("old process is still alive"),
  }, true, async () => { waits++; }), /has not stopped/);
  assert.equal(waits, 100);
});

test("stop errors are returned without restarting", async () => {
  await assert.rejects(restartSession(session.id, {
    session: async () => session,
    stop: async () => { throw new Error("stop failed"); },
    restart: async () => assert.fail("must not restart"),
  }, true), /stop failed/);
});

test("an already stopped session resumes without a stop call", async () => {
  assert.equal(await restartSession(session.id, {
    session: async () => ({ ...session, live: false }),
    stop: async () => assert.fail("already stopped"),
    restart: async () => session,
  }, true), session);
});

test("resume failures propagate so the stopped session remains recoverable", async () => {
  await assert.rejects(restartSession(session.id, {
    session: async () => ({ ...session, live: false }),
    stop: async () => assert.fail("already stopped"),
    restart: async () => { throw new Error("new executable unavailable"); },
  }, true), /new executable unavailable/);
});
