/**
 * Formatting inside a note body: a small markdown subset, kept as plain text so sync, merge and
 * search are unchanged. `**bold**`, `*italic*`, `__underline__`, `~~strike~~`, and links
 * (http(s):// or www.) found automatically. Markers stay in the text; editors show them faintly.
 * The same rules are implemented for the phone (format.dart); both are checked against
 * core/tests/fixtures/format.json.
 */

export type Span = {
  text: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strike?: boolean;
  /** The text is a link; `href` is what to open. */
  href?: string;
  /** A formatting marker (`**`, `__` ...), shown faintly. */
  marker?: boolean;
};

const LINK = /\bhttps?:\/\/[^\s<>"]+|\bwww\.[^\s<>"]+/g;
// Trailing punctuation and formatting markers are not part of a link: "**read https://a.io**".
const TRAIL = /[.,;:!?)\]'"*_~]+$/;
type Style = "bold" | "italic" | "underline" | "strike";
const MARKERS: [string, Style][] = [
  ["**", "bold"],
  ["__", "underline"],
  ["~~", "strike"],
  ["*", "italic"],
];

/** Links in one line: [start, end, href]. Trailing punctuation is not part of a link. */
function links(line: string): [number, number, string][] {
  const out: [number, number, string][] = [];
  for (const m of line.matchAll(LINK)) {
    const text = m[0].replace(TRAIL, "");
    if (!text) continue;
    const start = m.index!;
    out.push([start, start + text.length, text.startsWith("www.") ? `https://${text}` : text]);
  }
  return out;
}

function spansOfLine(line: string): Span[] {
  const n = line.length;
  const style: Partial<Record<Style, boolean>>[] = Array.from({ length: n }, () => ({}));
  const marker: boolean[] = new Array(n).fill(false);
  const href: (string | undefined)[] = new Array(n).fill(undefined);
  const ls = links(line);
  for (const [s, e, h] of ls) for (let i = s; i < e; i++) href[i] = h;
  const free = (i: number) => href[i] === undefined && !marker[i];

  for (const [mk, st] of MARKERS) {
    const k = mk.length;
    const at = (i: number) =>
      line.startsWith(mk, i) &&
      // a single * is not half of a ** pair, nor of a ***
      (k > 1 || (line[i - 1] !== "*" && line[i + 1] !== "*")) &&
      [...Array(k).keys()].every((d) => free(i + d));
    let i = 0;
    while (i < n) {
      if (at(i) && line[i + k] && !/\s/.test(line[i + k]) && line[i - 1] !== mk[0]) {
        // nearest closing marker with something in between and no space just before it
        let j = i + k + 1;
        while (j < n && !(at(j) && !/\s/.test(line[j - 1]) && line[j + k] !== mk[0])) j++;
        if (j < n) {
          for (let d = 0; d < k; d++) marker[i + d] = marker[j + d] = true;
          // Everything between, including inner markers, takes the style: **__x__** is bold underline.
          for (let c = i + k; c < j; c++) style[c][st] = true;
          i = j + k;
          continue;
        }
      }
      i++;
    }
  }
  const out: Span[] = [];
  for (let i = 0; i < n; i++) {
    const s: Span = { text: line[i], ...style[i] };
    if (marker[i]) s.marker = true;
    if (href[i]) s.href = href[i];
    const last = out[out.length - 1];
    if (last && same(last, s)) last.text += line[i];
    else out.push(s);
  }
  return out;
}

function same(a: Span, b: Span) {
  return (
    !!a.bold === !!b.bold &&
    !!a.italic === !!b.italic &&
    !!a.underline === !!b.underline &&
    !!a.strike === !!b.strike &&
    !!a.marker === !!b.marker &&
    a.href === b.href
  );
}

/** Spans for a whole body. Formatting never crosses a line break. */
export function format(text: string): Span[] {
  const out: Span[] = [];
  text.split("\n").forEach((line, i) => {
    if (i) {
      const last = out[out.length - 1];
      if (last && !last.bold && !last.italic && !last.underline && !last.strike && !last.marker && !last.href) last.text += "\n";
      else out.push({ text: "\n" });
    }
    for (const s of spansOfLine(line)) {
      const last = out[out.length - 1];
      if (last && same(last, s)) last.text += s.text;
      else out.push(s);
    }
  });
  return out;
}

/** The text without formatting markers, for titles and previews. */
export function plain(text: string): string {
  return format(text)
    .filter((s) => !s.marker)
    .map((s) => s.text)
    .join("");
}
