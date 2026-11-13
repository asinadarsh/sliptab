// Turning a key press into a hotkey string like "Super+Shift+N", and showing one to the user.

export type KeyPress = {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
};

export type Captured = { accel: string } | { error: string } | null;

const NAMED: Record<string, string> = {
  Space: "Space",
  Period: "Period",
  Comma: "Comma",
  Slash: "Slash",
  Semicolon: "Semicolon",
  Backquote: "Backquote",
  Minus: "Minus",
  Equal: "Equal",
};

function keyName(code: string): string | null {
  const m = /^(?:Key([A-Z])|Digit([0-9])|(F(?:[1-9]|1[0-9]|2[0-4])))$/.exec(code);
  if (m) return m[1] ?? m[2] ?? m[3];
  return NAMED[code] ?? null;
}

const MODIFIER_CODES = /^(Shift|Control|Alt|Meta|OS|Super)(Left|Right)?$/;

/**
 * null: only modifiers so far, keep waiting.
 * A hotkey needs a modifier other than Shift alone, or it would swallow normal typing
 * (Shift+N is just a capital N).
 */
export function captureHotkey(e: KeyPress): Captured {
  if (MODIFIER_CODES.test(e.code)) return null;
  const key = keyName(e.code);
  if (!key) return { error: "That key can't be used. Try a letter, number, F-key or Space." };
  const mods = [e.metaKey && "Super", e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift"].filter(
    (m): m is string => Boolean(m),
  );
  if (!mods.some((m) => m !== "Shift")) {
    return { error: "Add Ctrl, Alt or the Super/⌘ key, so normal typing still works." };
  }
  return { accel: [...mods, key].join("+") };
}

/** "Super+Shift+N" shown the way each OS names its keys. */
export function formatHotkey(accel: string, os: "macos" | "windows" | "linux"): string {
  const names: Record<string, Record<string, string>> = {
    macos: { Super: "⌘", Control: "⌃", Alt: "⌥", Shift: "⇧" },
    windows: { Super: "Win", Control: "Ctrl" },
    linux: { Super: "Super", Control: "Ctrl" },
  };
  const parts = accel.split("+").map((p) => names[os][p] ?? p);
  return os === "macos" ? parts.join("") : parts.join(" + ");
}
