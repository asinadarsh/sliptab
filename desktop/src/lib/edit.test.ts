import { test } from "node:test";
import assert from "node:assert/strict";
import { enterInItem, removeBox, toggleBox, toggleItemLine, toggleWrap } from "./edit.ts";

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

test("toggleBox ticks and unticks one line", () => {
  assert.equal(toggleBox("t\n- [ ] a\n  - [x] b", 1), "t\n- [x] a\n  - [x] b");
  assert.equal(toggleBox("t\n- [ ] a\n  - [x] b", 2), "t\n- [ ] a\n  - [ ] b");
  assert.equal(toggleBox("plain", 0), "plain");
});

test("removeBox takes the whole box", () => {
  assert.deepEqual(removeBox("a\n- [ ] milk", 8, true), { text: "a\nmilk", caret: 2 });
  assert.deepEqual(removeBox("a\n- [ ] milk", 2, false), { text: "a\nmilk", caret: 2 });
  assert.equal(removeBox("a\n- [ ] milk", 9, true), null); // inside "milk"
  assert.equal(removeBox("a\n- [ ] milk", 2, true), null); // before the box: joins lines
});

test("toggleItemLine adds and removes a box on the caret's line", () => {
  assert.deepEqual(toggleItemLine("a\nmilk", 4), { text: "a\n- [ ] milk", caret: 10 });
  assert.deepEqual(toggleItemLine("a\n- [x] milk", 10), { text: "a\nmilk", caret: 4 });
});

test("enterInItem continues the list and ends it on an empty item", () => {
  assert.deepEqual(enterInItem("- [ ] milk", 10), { text: "- [ ] milk\n- [ ] ", caret: 17 });
  assert.deepEqual(enterInItem("- [ ] milk\n- [ ] ", 17), { text: "- [ ] milk\n", caret: 11 });
  assert.equal(enterInItem("plain", 5), null);
});
