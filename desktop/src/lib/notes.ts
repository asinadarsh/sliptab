import { plain } from "./format.ts";
// Pure helpers shared by the three panel layouts. No DOM, tested with `node --test`.

export type Note = {
  id: string;
  title: string;
  body: string;
  pinned: boolean;
  created_at: number;
  updated_at: number;
  color: string | null;
  /** Place of a pinned note's tab, smallest first; absent in older notes. */
  pin_order?: number | null;
  /** Present when the note is a saved post. */
  link?: Link;
  /** Images and recordings, removed ones included (deleted_at set). */
  attachments?: Attachment[];
};

export type Attachment = {
  id: string;
  kind: "image" | "audio" | "file";
  mime: string;
  bytes: number;
  sha256: string;
  name?: string | null;
  width?: number | null;
  height?: number | null;
  duration_ms?: number | null;
  created_at: number;
  deleted_at?: number | null;
};

/** Attachments still on the note. */
export const liveAttachments = (n: Note) => (n.attachments ?? []).filter((a) => !a.deleted_at);

export type Link = {
  url: string;
  platform: "instagram" | "threads" | "x" | "youtube" | "web";
  title: string | null;
  author: string | null;
  text: string | null;
  thumbnail: string | null;
  folder: string | null;
  done: boolean;
  opened_at: number | null;
};

/** Names and brand colours, the same as the phone app. */
export const PLATFORMS: Record<string, { name: string; color: string; mark: string }> = {
  instagram: { name: "Instagram", color: "#e1306c", mark: "IG" },
  threads: { name: "Threads", color: "#101010", mark: "@" },
  x: { name: "X", color: "#101010", mark: "𝕏" },
  youtube: { name: "YouTube", color: "#ff0033", mark: "▶" },
};

const host = (url: string) => {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return url;
  }
};

export const platformName = (l: Link) => PLATFORMS[l.platform]?.name ?? host(l.url);

/** A save's card title: the page's title once fetched, else the words shared with the link,
 *  else the link without "https://www.". Same rule as the phone. */
export function saveTitle(n: Note): string {
  const l = n.link!;
  if (l.title?.trim()) return l.title.trim();
  const words = plain(n.body).split(/\s+/).filter((w) => !/^https?:\/\//.test(w)).join(" ").trim();
  if (words) return words;
  try {
    const u = new URL(l.url);
    return host(l.url) + (u.pathname === "/" ? "" : u.pathname);
  } catch {
    return l.url;
  }
}

/** Typed or pasted text that is nothing but a link is saved as a post; anything else is a note. */
export const isOnlyLink = (text: string) => /^https?:\/\/\S+$/.test(text.trim());

/** Distinct, readable on light and dark. New pins take the first colour no other pin uses. */
export const PALETTE = ["#f5a524", "#3b82f6", "#ec4899", "#22c55e", "#8b5cf6", "#14b8a6", "#ef4444", "#64748b"];

export function nextColor(notes: Note[]): string {
  const used = new Set(notes.filter((n) => n.pinned && n.color).map((n) => n.color));
  return PALETTE.find((c) => !used.has(c)) ?? PALETTE[notes.filter((n) => n.pinned).length % PALETTE.length];
}

/** Six note tabs at most; more pins collapse into one "+N" tab. Ordered by creation so tabs never jump. */
export const MAX_NOTE_TABS = 6;

/** Pinned notes in their tab order (as pinned, or as dragged); older notes by when they were made. */
export function byPinOrder(pinned: Note[]): Note[] {
  const key = (n: Note) => n.pin_order ?? n.created_at;
  return [...pinned].sort((a, b) => key(a) - key(b) || a.created_at - b.created_at);
}
export function edgeTabs(notes: Note[]): { tabs: Note[]; more: number } {
  const pinned = byPinOrder(notes.filter((n) => n.pinned));
  return { tabs: pinned.slice(0, MAX_NOTE_TABS), more: Math.max(0, pinned.length - MAX_NOTE_TABS) };
}

/** Body without the title line, whitespace squashed. */
export function preview(n: Note): string {
  const lines = plain(n.body).split("\n");
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
    .map((m) => ({ text: plain(m[2]), done: m[1] !== " " }));
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
    { name: "Pinned", notes: byPinOrder(notes.filter((n) => n.pinned)) },
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

/** "2 images · 1 audio · 3 files", or "" when the note has none. */
export function attachSummary(n: Note): string {
  const live = liveAttachments(n);
  const count = (k: string) => live.filter((a) => a.kind === k).length;
  const [i, v, f] = [count("image"), count("audio"), count("file")];
  return [i && `${i} image${i > 1 ? "s" : ""}`, v && `${v} audio`, f && `${f} file${f > 1 ? "s" : ""}`].filter(Boolean).join(" · ");
}
