import assert from "node:assert/strict";
import test from "node:test";
import { matchesSessionFilter } from "../src/sessionFilters.ts";

const statuses = ["needs_you", "running", "idle", "done", "error"];

test("held sessions appear only in On hold, regardless of process status", () => {
  for (const status of statuses) {
    const session = { status, onHold: true };
    for (const filter of ["all", "attention", "running"]) {
      assert.equal(matchesSessionFilter(session, filter), false, `${status} in ${filter}`);
    }
    assert.equal(matchesSessionFilter(session, "on_hold"), true);
  }
});

test("taking a session off hold restores filters based on its latest status", () => {
  for (const status of statuses) {
    const session = { status, onHold: false };
    assert.equal(matchesSessionFilter(session, "all"), true);
    assert.equal(matchesSessionFilter(session, "on_hold"), false);
    assert.equal(matchesSessionFilter(session, "attention"), ["needs_you", "error"].includes(status));
    assert.equal(matchesSessionFilter(session, "running"), status === "running");
  }
});

test("sessions from an older daemon without a hold flag remain visible", () => {
  assert.equal(matchesSessionFilter({ status: "running" }, "all"), true);
  assert.equal(matchesSessionFilter({ status: "running" }, "running"), true);
  assert.equal(matchesSessionFilter({ status: "running" }, "on_hold"), false);
});
