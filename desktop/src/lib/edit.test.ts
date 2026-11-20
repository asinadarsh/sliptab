import { test } from "node:test";
import assert from "node:assert/strict";
import { toggleWrap } from "./edit.ts";

test("bold a selection, then unbold it", () => {
  const a = toggleWrap("buy milk now", 4, 8, "**");
  assert.deepEqual(a, { text: "buy **milk** now", start: 6, end: 10 });
  assert.deepEqual(toggleWrap(a.text, a.start, a.end, "**"), { text: "buy milk now", start: 4, end: 8 });
});

test("a selection that includes the markers unwraps too", () => {
  assert.deepEqual(toggleWrap("x __u__ y", 2, 7, "__"), { text: "x u y", start: 2, end: 3 });
});

test("spaces stay outside the markers", () => {
  assert.equal(toggleWrap("a word here", 1, 7, "**").text, "a **word** here");
});

test("no selection inserts a pair with the caret inside", () => {
  assert.deepEqual(toggleWrap("ab", 1, 1, "~~"), { text: "a~~~~b", start: 3, end: 3 });
});
