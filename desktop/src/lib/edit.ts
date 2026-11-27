/**
 * Editing helpers for formatted text, kept pure so they can be tested without a DOM.
 */

/** Wraps [start, end) in `mk`, or removes `mk` if the selection is already wrapped. With nothing
 *  selected it inserts an empty pair and puts the caret between. */
export function toggleWrap(text: string, start: number, end: number, mk: string): { text: string; start: number; end: number } {
  const k = mk.length;
  const before = text.slice(start - k, start);
  const after = text.slice(end, end + k);
  if (before === mk && after === mk) {
    return { text: text.slice(0, start - k) + text.slice(start, end) + text.slice(end + k), start: start - k, end: end - k };
  }
  const inner = text.slice(start, end);
  if (inner.length >= 2 * k && inner.startsWith(mk) && inner.endsWith(mk)) {
    return { text: text.slice(0, start) + inner.slice(k, -k) + text.slice(end), start, end: end - 2 * k };
  }
  // Keep surrounding spaces outside the markers: "**word** " not "**word **".
  const lead = inner.length - inner.trimStart().length;
  const trail = inner.length - inner.trimEnd().length;
  const s = start + lead;
  const e = Math.max(s, end - trail);
  return { text: text.slice(0, s) + mk + text.slice(s, e) + mk + text.slice(e), start: s + k, end: e + k };
}

const ITEM = /^(\s*)- \[([ xX])\] /;

/** Ticks or unticks the checklist box on line `line` (0-based). */
export function toggleBox(text: string, line: number): string {
  const lines = text.split("\n");
  const m = ITEM.exec(lines[line] ?? "");
  if (!m) return text;
  const at = m[1].length + 3;
  lines[line] = lines[line].slice(0, at) + (m[2] === " " ? "x" : " ") + lines[line].slice(at + 1);
  return lines.join("\n");
}

/** A checklist box ("- [ ] ", drawn as one box) is removed whole by a backspace just after it or
 *  a delete just before it, so no stray "- [" is left. Returns null when the key is not at a box. */
export function removeBox(text: string, caret: number, back: boolean): { text: string; caret: number } | null {
  const start = text.lastIndexOf("\n", caret - 1) + 1;
  const m = ITEM.exec(text.slice(start));
  if (!m) return null;
  const box = start + m[1].length;
  if (back ? caret <= box || caret > box + 6 : caret < box || caret >= box + 6) return null;
  return { text: text.slice(0, box) + text.slice(box + 6), caret: box };
}

/** Puts a checklist box at the start of the caret's line, or takes it away if there is one. */
export function toggleItemLine(text: string, caret: number): { text: string; caret: number } {
  const start = text.lastIndexOf("\n", caret - 1) + 1;
  const m = ITEM.exec(text.slice(start));
  if (m) {
    const box = start + m[1].length;
    return { text: text.slice(0, box) + text.slice(box + 6), caret: Math.max(box, caret - 6) };
  }
  return { text: text.slice(0, start) + "- [ ] " + text.slice(start), caret: caret + 6 };
}

/** Enter on a checklist item starts the next item; Enter on an empty item removes its box and
 *  ends the list. Returns null when the caret's line is not an item (a plain newline then). */
export function enterInItem(text: string, caret: number): { text: string; caret: number } | null {
  const start = text.lastIndexOf("\n", caret - 1) + 1;
  const m = ITEM.exec(text.slice(start));
  if (!m) return null;
  const box = start + m[1].length;
  if (caret < box + 6) return null;
  let end = text.indexOf("\n", start);
  if (end < 0) end = text.length;
  if (!text.slice(box + 6, end).trim()) return { text: text.slice(0, box) + text.slice(box + 6), caret: box };
  const next = "\n" + m[1] + "- [ ] ";
  return { text: text.slice(0, caret) + next + text.slice(caret), caret: caret + next.length };
}

const BULLET = /^(\s*)([-*•]|\d{1,3}[.)]) (?!\[[ xX]\] )/;

/** Enter in a bullet ("- ", "* ") or numbered ("1. ", "1) ") line continues the list with the
 *  next marker; Enter on an empty item ends the list. Null when the line isn't a list item. */
export function enterInList(text: string, caret: number): { text: string; caret: number } | null {
  const start = text.lastIndexOf("\n", caret - 1) + 1;
  const m = BULLET.exec(text.slice(start));
  if (!m) return null;
  const body = start + m[0].length;
  if (caret < body) return null;
  let end = text.indexOf("\n", start);
  if (end < 0) end = text.length;
  if (!text.slice(body, end).trim()) return { text: text.slice(0, start) + text.slice(body), caret: start };
  const num = /^(\d+)([.)])$/.exec(m[2]);
  const marker = num ? `${+num[1] + 1}${num[2]}` : m[2];
  const next = `\n${m[1]}${marker} `;
  return { text: text.slice(0, caret) + next + text.slice(caret), caret: caret + next.length };
}

/** Typed arrows: "->" "<-" "=>" "<->" become → ← ⇒ ↔, and "-->" (the first "-" already part of
 *  what was typed) is the same → . `typed` is the character just inserted before `caret`.
 *  Null when nothing changes. Not inside links or code-looking text: only after a space or at
 *  the line start, and only when the user types it (a paste is left alone). */
export function arrow(text: string, caret: number, typed: string): { text: string; caret: number } | null {
  if (typed !== ">" && typed !== "-") return null;
  const before = text.slice(Math.max(0, caret - 3), caret);
  const rules: [string, string][] = [["<->", "↔"], ["-->", "→"], ["->", "→"], ["=>", "⇒"], ["<-", "←"]];
  for (const [from, to] of rules) {
    if (!before.endsWith(from)) continue;
    const at = caret - from.length;
    const prev = text[at - 1];
    if (prev !== undefined && !/\s/.test(prev)) return null; // a->b inside a word or URL stays
    return { text: text.slice(0, at) + to + text.slice(caret), caret: at + to.length };
  }
  return null;
}
