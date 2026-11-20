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
