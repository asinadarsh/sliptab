import { test } from "node:test";
import assert from "node:assert/strict";
import { createEdge, type Mode, type Reason, type Timers } from "./hover.ts";

// A manual clock so the tests control time exactly.
function setup() {
  let now = 0;
  let next = 1;
  const queue = new Map<number, { at: number; fn: () => void }>();
  const timers: Timers = {
    set(fn, ms) {
      const id = next++;
      queue.set(id, { at: now + ms, fn });
      return id;
    },
    clear(h) {
      queue.delete(h as number);
    },
  };
  const advance = (ms: number) => {
    now += ms;
    for (const [id, t] of [...queue].sort((a, b) => a[1].at - b[1].at)) {
      if (t.at <= now && queue.has(id)) {
        queue.delete(id);
        t.fn();
      }
    }
  };
  const changes: [Mode, Reason][] = [];
  const edge = createEdge({ hoverDelay: 300, leaveDelay: 400, timers, onChange: (m, r) => changes.push([m, r]) });
  return { edge, advance, changes };
}

test("a deliberate hover expands after the delay", () => {
  const { edge, advance, changes } = setup();
  edge.pointerEnter();
  advance(299);
  assert.equal(edge.mode, "collapsed");
  advance(1);
  assert.deepEqual(changes, [["expanded", "hover"]]);
});

test("a passing mouse does not expand", () => {
  const { edge, advance, changes } = setup();
  edge.pointerEnter();
  advance(150);
  edge.pointerLeave();
  advance(1000);
  assert.deepEqual(changes, []);
});

test("leaving collapses after a grace period, re-entering cancels it", () => {
  const { edge, advance, changes } = setup();
  edge.click();
  edge.pointerLeave();
  advance(200);
  edge.pointerEnter();
  advance(1000);
  assert.equal(edge.mode, "expanded");
  edge.pointerLeave();
  advance(400);
  assert.deepEqual(changes, [["expanded", "click"], ["collapsed", "leave"]]);
});

test("escape collapses at once and the hotkey toggles", () => {
  const { edge, changes } = setup();
  edge.hotkey();
  edge.escape();
  edge.hotkey();
  edge.hotkey();
  assert.deepEqual(changes, [
    ["expanded", "hotkey"],
    ["collapsed", "escape"],
    ["expanded", "hotkey"],
    ["collapsed", "hotkey"],
  ]);
});

test("pause stops hover but not click", () => {
  const { edge, advance, changes } = setup();
  edge.setPaused(true);
  edge.pointerEnter();
  advance(1000);
  assert.equal(edge.mode, "collapsed");
  edge.click();
  assert.deepEqual(changes, [["expanded", "click"]]);
});

test("pausing mid-hover cancels the pending expand", () => {
  const { edge, advance } = setup();
  edge.pointerEnter();
  advance(100);
  edge.setPaused(true);
  advance(1000);
  assert.equal(edge.mode, "collapsed");
});

test("movement starts hover even when the enter event never arrived", () => {
  const { edge, advance, changes } = setup();
  edge.pointerMove();
  advance(200);
  edge.pointerMove(); // further movement must not restart the timer
  advance(400);
  assert.deepEqual(changes, [["expanded", "hover"]]);
});

test("leave after movement still cancels a passing mouse", () => {
  const { edge, advance, changes } = setup();
  edge.pointerMove();
  advance(100);
  edge.pointerLeave();
  advance(1000);
  assert.deepEqual(changes, []);
});
