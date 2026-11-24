import assert from "node:assert/strict";
import test from "node:test";
import {
  LABEL_COLOR_NAMES,
  labelHex,
  labelsBySession,
  nextLabelColor,
  withSessionLabels,
} from "../src/sessionLabels.ts";

const bug = { id: 1, name: "Bug", color: "red" };
const client = { id: 2, name: "Client work", color: "blue" };
const review = { id: 3, name: "Waiting on review", color: "yellow" };
const book = { labels: [bug, client, review], sessions: { a: [3, 1], b: [2], gone: [9] } };

test("a session's labels come in the labels' own order, without ones that no longer exist", () => {
  const by = labelsBySession(book);
  assert.deepEqual(by.get("a"), [bug, review]);
  assert.deepEqual(by.get("b"), [client]);
  assert.equal(by.has("gone"), false);
  assert.equal(by.has("never-labelled"), false);
});

test("changing a session's labels leaves the others and the book itself alone", () => {
  const next = withSessionLabels(book, "b", [3, 2, 42]);
  assert.deepEqual(next.sessions.b, [2, 3]);
  assert.deepEqual(next.sessions.a, [3, 1]);
  assert.deepEqual(book.sessions.b, [2]);
  // A session left with none is dropped.
  assert.equal("a" in withSessionLabels(book, "a", []).sessions, false);
});

test("a new label is offered a colour that isn't in use yet", () => {
  assert.equal(nextLabelColor([]), LABEL_COLOR_NAMES[0]);
  assert.equal(nextLabelColor([bug, client, review]), "orange");
  const all = LABEL_COLOR_NAMES.map((color) => ({ color }));
  assert.ok(LABEL_COLOR_NAMES.includes(nextLabelColor(all)));
});

test("a colour this version doesn't know shows as grey", () => {
  assert.equal(labelHex("teal"), labelHex("gray"));
  assert.notEqual(labelHex("red"), labelHex("gray"));
});
