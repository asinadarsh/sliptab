import { test } from "node:test";
import assert from "node:assert/strict";
import { captureHotkey, formatHotkey, type KeyPress } from "./accel.ts";

const press = (code: string, mods: Partial<KeyPress> = {}): KeyPress => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

test("captures the default and keeps a fixed modifier order", () => {
  assert.deepEqual(captureHotkey(press("KeyN", { shiftKey: true, metaKey: true })), { accel: "Super+Shift+N" });
  assert.deepEqual(captureHotkey(press("KeyN", { altKey: true, ctrlKey: true })), { accel: "Control+Alt+N" });
  assert.deepEqual(captureHotkey(press("Space", { ctrlKey: true })), { accel: "Control+Space" });
  assert.deepEqual(captureHotkey(press("F7", { altKey: true })), { accel: "Alt+F7" });
  assert.deepEqual(captureHotkey(press("Digit3", { metaKey: true })), { accel: "Super+3" });
});

test("waits while only modifiers are held", () => {
  for (const code of ["ShiftLeft", "ControlRight", "AltLeft", "MetaLeft", "OSLeft"]) {
    assert.equal(captureHotkey(press(code, { shiftKey: true })), null);
  }
});

test("rejects combos that would break typing or keys we cannot bind", () => {
  assert.ok("error" in captureHotkey(press("KeyN"))!);
  assert.ok("error" in captureHotkey(press("KeyN", { shiftKey: true }))!);
  assert.ok("error" in captureHotkey(press("ArrowUp", { ctrlKey: true }))!);
  assert.ok("error" in captureHotkey(press("F25", { ctrlKey: true }))!);
});

test("formats per OS", () => {
  assert.equal(formatHotkey("Super+Shift+N", "linux"), "Super + Shift + N");
  assert.equal(formatHotkey("Super+Shift+N", "windows"), "Win + Shift + N");
  assert.equal(formatHotkey("Control+Alt+N", "macos"), "⌃⌥N");
});
