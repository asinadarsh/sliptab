import { test } from "node:test";
import assert from "node:assert/strict";
import { arrow, enterInItem, enterInList, removeBox, toggleBox, toggleItemLine, toggleWrap } from "./edit.ts";

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


test("Enter continues bullet and numbered lists, and an empty item ends them", () => {
  assert.deepEqual(enterInList("- milk", 6), { text: "- milk\n- ", caret: 9 });
  assert.deepEqual(enterInList("  * eggs", 8), { text: "  * eggs\n  * ", caret: 13 });
  assert.deepEqual(enterInList("9. call mum", 11), { text: "9. call mum\n10. ", caret: 16 });
  assert.deepEqual(enterInList("1) a", 4), { text: "1) a\n2) ", caret: 8 });
  assert.deepEqual(enterInList("a\n- ", 4), { text: "a\n", caret: 2 });
  assert.equal(enterInList("plain", 5), null);
  assert.equal(enterInList("- [ ] task", 10), null, "checklists have their own rule");
  assert.equal(enterInList("-milk", 5), null);
});

test("typed arrows become arrow characters", () => {
  assert.deepEqual(arrow("go ->", 5, ">"), { text: "go →", caret: 4 });
  assert.deepEqual(arrow("go -->", 6, ">"), { text: "go →", caret: 4 });
  assert.deepEqual(arrow("->", 2, ">"), { text: "→", caret: 1 });
  assert.deepEqual(arrow("a <-", 4, "-"), { text: "a ←", caret: 3 });
  assert.deepEqual(arrow("x <->", 5, ">"), { text: "x ↔", caret: 3 });
  assert.deepEqual(arrow("so =>", 5, ">"), { text: "so ⇒", caret: 4 });
  assert.equal(arrow("a->b", 3, ">"), null, "inside a word");
  assert.equal(arrow("x >", 3, ">"), null);
});
