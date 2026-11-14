// Pure helpers shared by the three panel layouts. No DOM, tested with `node --test`.

export type Note = {
  id: string;
  title: string;
  body: string;
  pinned: boolean;
  created_at: number;
  updated_at: number;
  color: string | null;
};

/** Distinct, readable on light and dark. New pins take the first colour no other pin uses. */
export const PALETTE = ["#f5a524", "#3b82f6", "#ec4899", "#22c55e", "#8b5cf6", "#14b8a6", "#ef4444", "#64748b"];

export function nextColor(notes: Note[]): string {
  const used = new Set(notes.filter((n) => n.pinned && n.color).map((n) => n.color));
  return PALETTE.find((c) => !used.has(c)) ?? PALETTE[notes.filter((n) => n.pinned).length % PALETTE.length];
}

/** Six note tabs at most; more pins collapse into one "+N" tab. Ordered by creation so tabs never jump. */
export const MAX_NOTE_TABS = 6;
export function edgeTabs(notes: Note[]): { tabs: Note[]; more: number } {
  const pinned = notes.filter((n) => n.pinned).sort((a, b) => a.created_at - b.created_at);
  return { tabs: pinned.slice(0, MAX_NOTE_TABS), more: Math.max(0, pinned.length - MAX_NOTE_TABS) };
}

/** Body without the title line, whitespace squashed. */
export function preview(n: Note): string {
  const lines = n.body.split("\n");
  const first = lines.findIndex((l) => l.trim() !== "");
  return lines
    .slice(first + 1)
    .map((l) => l.replace(/^\s*- \[[ xX]\] /, ""))
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
}

export type ChecklistItem = { text: string; done: boolean };
export function checklistItems(n: Note): ChecklistItem[] {
  return n.body
    .split("\n")
    .map((l) => /^\s*- \[([ xX])\] (.*)$/.exec(l))
    .filter((m): m is RegExpExecArray => m !== null)
    .map((m) => ({ text: m[2], done: m[1] !== " " }));
}

export function progress(n: Note): { done: number; total: number } | null {
  const items = checklistItems(n);
  return items.length ? { done: items.filter((i) => i.done).length, total: items.length } : null;
}

export type Section = { name: "Pinned" | "Today" | "Earlier" | "Results"; notes: Note[] };

/** Same order for every layout: pinned, today, earlier. A search shows one flat "Results" section. */
export function sections(notes: Note[], searching: boolean, now = Date.now()): Section[] {
  if (searching) return notes.length ? [{ name: "Results", notes }] : [];
  const midnight = new Date(now);
  midnight.setHours(0, 0, 0, 0);
  const out: Section[] = [
    { name: "Pinned", notes: notes.filter((n) => n.pinned) },
    { name: "Today", notes: notes.filter((n) => !n.pinned && n.updated_at >= midnight.getTime()) },
    { name: "Earlier", notes: notes.filter((n) => !n.pinned && n.updated_at < midnight.getTime()) },
  ];
  return out.filter((s) => s.notes.length);
}

export function ago(ms: number, now = Date.now()): string {
  const s = Math.round((now - ms) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 2 * 86400) return "Yesterday";
  if (s < 7 * 86400) return new Date(ms).toLocaleDateString(undefined, { weekday: "short" });
  return new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}
