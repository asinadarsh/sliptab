import { test } from "node:test";
import assert from "node:assert/strict";
import { checklistItems, edgeTabs, nextColor, PALETTE, preview, progress, sections, type Note } from "./notes.ts";

let t = 0;
const note = (body: string, extra: Partial<Note> = {}): Note => ({
  id: `n${++t}`,
  title: body.split("\n")[0],
  body,
  pinned: false,
  created_at: t,
  updated_at: t,
  color: null,
  ...extra,
});

test("preview drops the title line and checklist markers", () => {
  assert.equal(preview(note("Groceries\n- [ ] milk\n- [x] atta\n\n  curd ")), "milk atta curd");
  assert.equal(preview(note("\n\nOnly a title")), "");
});

test("checklist progress", () => {
  const n = note("List\n- [x] a\n- [ ] b\n- [X] c\nnot an item");
  assert.deepEqual(checklistItems(n).map((i) => i.done), [true, false, true]);
  assert.deepEqual(progress(n), { done: 2, total: 3 });
  assert.equal(progress(note("plain")), null);
});

test("new pins take the first unused colour", () => {
  const a = note("a", { pinned: true, color: PALETTE[0] });
  const b = note("b", { pinned: false, color: PALETTE[1] }); // unpinned colours are free again
  assert.equal(nextColor([a, b]), PALETTE[1]);
  const all = PALETTE.map((c) => note(c, { pinned: true, color: c }));
  assert.ok(PALETTE.includes(nextColor(all)));
});

test("edge tabs are stable by creation order and capped at six plus more", () => {
  const pins = Array.from({ length: 8 }, (_, i) => note(`p${i}`, { pinned: true, updated_at: 100 - i }));
  const { tabs, more } = edgeTabs([note("x"), ...pins.slice().reverse()]);
  assert.deepEqual(tabs.map((n) => n.title), ["p0", "p1", "p2", "p3", "p4", "p5"]);
  assert.equal(more, 2);
});

test("sections: pinned, today, earlier; search is one flat list", () => {
  const now = new Date(2026, 10, 14, 12).getTime();
  const notes = [
    note("pin", { pinned: true, updated_at: now - 9e9 }),
    note("today", { updated_at: now - 3600e3 }),
    note("old", { updated_at: now - 2 * 86400e3 }),
  ];
  assert.deepEqual(sections(notes, false, now).map((s) => [s.name, s.notes.length]), [
    ["Pinned", 1],
    ["Today", 1],
    ["Earlier", 1],
  ]);
  assert.deepEqual(sections(notes, true, now).map((s) => s.name), ["Results"]);
  assert.deepEqual(sections([], true, now), []);
});
