<!-- The note editor: plain text with live formatting. The text (with its **markers**) is the value;
     on every edit it is re-rendered as styled spans and the caret is put back by character offset.
     Markers stay visible, faintly, so text and caret positions are the same as in a textarea. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { format, type Span } from "./format";
  import { arrow, enterInItem, enterInList, removeBox, toggleBox, toggleItemLine, toggleWrap } from "./edit";

  let {
    value = $bindable(""),
    oninput,
    onopenlink,
    onfiles,
    el = $bindable(),
  }: {
    value: string;
    oninput?: () => void;
    onopenlink?: (href: string) => void;
    /** Images or audio pasted or dropped into the editor. */
    onfiles?: (files: File[]) => void;
    el?: HTMLDivElement;
  } = $props();

  let composing = false;
  let lastRendered: string | null = null;

  // ---- undo: own history, because re-rendering the DOM clears the browser's.
  type Snap = { text: string; caret: number };
  let undo: Snap[] = [];
  let redo: Snap[] = [];
  let lastPush = 0;
  function remember(text: string, caret: number, force = false) {
    const now = performance.now();
    if (!force && undo.length && now - lastPush < 700) undo[undo.length - 1] = { text, caret };
    else undo.push({ text, caret });
    if (undo.length > 200) undo.shift();
    lastPush = now;
    redo = [];
  }

  const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  /** Long links show as "site.com/path/…" with the rest hidden (still in the text, at size 0,
   *  so caret offsets stay exact) except on the line being edited. */
  const LINK_SHOWN = 42;
  function span(s: Span): string {
    const cls = [s.bold && "b", s.italic && "i", s.underline && "u", s.strike && "s", s.marker && "m", s.href && "lnk"].filter(Boolean).join(" ");
    if (s.href) {
      const attr = ` data-href="${esc(s.href)}"`;
      if (s.text.length <= LINK_SHOWN) return `<span class="${cls}"${attr}>${esc(s.text)}</span>`;
      return `<span class="${cls}"${attr}>${esc(s.text.slice(0, LINK_SHOWN))}<span class="lnk-more">${esc(s.text.slice(LINK_SHOWN))}</span></span>`;
    }
    return cls ? `<span class="${cls}">${esc(s.text)}</span>` : esc(s.text);
  }

  // A link under the pointer gets a small card: its site, Open and Copy.
  let linkCard: { href: string; x: number; y: number } | null = $state(null);
  let cardTimer: ReturnType<typeof setTimeout> | undefined;
  let copied = $state(false);
  function onOver(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest<HTMLElement>("[data-href]");
    if (!a) return;
    clearTimeout(cardTimer);
    const r = a.getBoundingClientRect();
    const href = a.dataset.href!;
    if (linkCard?.href === href) return;
    copied = false;
    linkCard = { href, x: Math.min(r.left, window.innerWidth - 250), y: r.bottom + 6 };
  }
  function onOut(e: MouseEvent) {
    const to = e.relatedTarget as HTMLElement | null;
    if (to?.closest("[data-href], .link-card")) return;
    clearTimeout(cardTimer);
    cardTimer = setTimeout(() => (linkCard = null), 250);
  }
  async function copyLink(href: string) {
    try {
      await invoke("clip_write", { text: href });
    } catch {
      await navigator.clipboard.writeText(href);
    }
    copied = true;
    setTimeout(() => (linkCard = null), 900);
  }
  const site = (href: string) => {
    try {
      const u = new URL(href);
      return u.hostname.replace(/^www\./, "") + (u.pathname.length > 1 ? u.pathname : "");
    } catch {
      return href;
    }
  };
  /** One <span class="line"> per line (the newline itself stays plain text between them), so the
   *  line with the caret can show its markers while every other line hides them. */
  function html(text: string): string {
    const out = text.split("\n").map((line, i) => {
      // A checklist item's "- [ ]" is drawn as one box (the text stays, at size 0, so offsets
      // match). Its trailing space stays normal-size, so a caret after the box is visible.
      // A ticked item's text is struck through.
      const m = /^(\s*)(- \[([ xX])\] )/.exec(line);
      if (!m) return `<span class="line">${format(line).map(span).join("")}</span>`;
      const done = m[3] !== " ";
      const rest = format(line.slice(m[0].length)).map(span).join("");
      return `<span class="line item${done ? " done" : ""}">${esc(m[1])}<span class="box" data-line="${i}">${m[2].slice(0, 5)}</span><span class="txt"> ${rest}</span></span>`;
    });
    // A trailing newline needs something after it to be drawn as an empty line.
    return out.join("\n") + (text.endsWith("\n") || text === "" ? "<br>" : "");
  }

  /** Marks the line holding the caret, whose markers are shown. */
  function markCurrentLine() {
    if (!el) return;
    const sel = getSelection();
    let node: Node | null = sel?.rangeCount && el.contains(sel.anchorNode) ? sel.anchorNode : null;
    while (node && !(node instanceof HTMLElement && node.classList.contains("line"))) node = node.parentNode;
    for (const l of el.querySelectorAll(".line.cur")) if (l !== node) l.classList.remove("cur");
    (node as HTMLElement | null)?.classList.add("cur");
  }
  $effect(() => {
    const f = () => markCurrentLine();
    document.addEventListener("selectionchange", f);
    return () => document.removeEventListener("selectionchange", f);
  });

  /** Characters from the start of the editor to (node, offset). */
  function offsetOf(node: Node, offset: number): number {
    if (!el) return 0;
    const r = document.createRange();
    r.setStart(el, 0);
    r.setEnd(node, offset);
    return r.toString().length;
  }
  function caret(): { start: number; end: number } | null {
    const sel = getSelection();
    if (!el || !sel || !sel.rangeCount) return null;
    const r = sel.getRangeAt(0);
    if (!el.contains(r.startContainer)) return null;
    return { start: offsetOf(r.startContainer, r.startOffset), end: offsetOf(r.endContainer, r.endOffset) };
  }
  function pointAt(n: number): [Node, number] {
    const walk = document.createTreeWalker(el!, NodeFilter.SHOW_TEXT);
    let left = n;
    let last: Text | null = null;
    for (let t = walk.nextNode() as Text | null; t; t = walk.nextNode() as Text | null) {
      if (left <= t.length) return [t, left];
      left -= t.length;
      last = t;
    }
    return last ? [last, last.length] : [el!, 0];
  }
  function setCaret(start: number, end = start) {
    const sel = getSelection();
    if (!el || !sel) return;
    const r = document.createRange();
    r.setStart(...pointAt(start));
    r.setEnd(...pointAt(end));
    sel.removeAllRanges();
    sel.addRange(r);
  }
  function render(text: string, keep: { start: number; end: number } | null) {
    if (!el) return;
    el.innerHTML = html(text);
    lastRendered = text;
    if (keep) setCaret(keep.start, keep.end);
    markCurrentLine();
  }

  // Outside changes to `value` (another note opened, a sync) re-render; our own edits already did.
  $effect(() => {
    if (el && value !== lastRendered && !composing) {
      const focused = document.activeElement === el;
      render(value, focused ? caret() : null);
      if (!focused) {
        undo = [{ text: value, caret: value.length }];
        redo = [];
      }
    }
  });

  function read(): string {
    // textContent: the spans hold the text; the placeholder <br> adds nothing.
    return el?.textContent ?? "";
  }
  function changed(text: string, at: { start: number; end: number } | null) {
    value = text;
    render(text, at);
    oninput?.();
  }
  function onInput(e?: Event) {
    if (composing) return;
    let at = caret();
    let text = read();
    // "->" becomes → as it is typed (a typed character only, never a paste).
    const ie = e as InputEvent | undefined;
    if (ie?.inputType === "insertText" && ie.data && at && at.start === at.end) {
      const a = arrow(text, at.start, ie.data.slice(-1));
      if (a) {
        text = a.text;
        at = { start: a.caret, end: a.caret };
      }
    }
    remember(text, at?.start ?? text.length);
    changed(text, at);
  }

  function apply(mk: string) {
    const at = caret() ?? { start: value.length, end: value.length };
    const r = toggleWrap(value, at.start, at.end, mk);
    remember(value, at.start, true);
    remember(r.text, r.end, true);
    changed(r.text, { start: r.start, end: r.end });
  }
  export function bold() { apply("**"); }
  export function italic() { apply("*"); }
  export function underline() { apply("__"); }
  export function strike() { apply("~~"); }
  // For the app's own right-click menu.
  export function selection(): string {
    const at = caret();
    return at ? value.slice(at.start, at.end) : "";
  }
  /** Replaces the selection (or inserts at the caret). */
  export function insert(s: string) {
    el?.focus();
    insertText(s);
  }
  export function selectAll() {
    el?.focus();
    setCaret(0, value.length);
  }

  /** A click below the text: caret to the end, ready to type. */
  export function focusEnd() {
    el?.focus();
    setCaret(value.length);
    markCurrentLine();
  }
  export function checklist() {
    const at = caret() ?? { start: value.length, end: value.length };
    const r = toggleItemLine(value, at.start);
    remember(value, at.start, true);
    remember(r.text, r.caret, true);
    changed(r.text, { start: r.caret, end: r.caret });
  }

  function insertText(s: string) {
    const at = caret() ?? { start: value.length, end: value.length };
    const text = value.slice(0, at.start) + s + value.slice(at.end);
    remember(text, at.start + s.length, true);
    changed(text, { start: at.start + s.length, end: at.start + s.length });
  }

  // undo's last entry is always the current text.
  function show(snap: Snap) {
    value = snap.text;
    render(snap.text, { start: snap.caret, end: snap.caret });
    oninput?.();
  }
  function undoStep() {
    if (undo.length < 2) return;
    redo.push(undo.pop()!);
    show(undo[undo.length - 1]);
  }
  function redoStep() {
    const s = redo.pop();
    if (!s) return;
    undo.push(s);
    show(s);
  }

  function onKeydown(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if ((e.key === "Backspace" || e.key === "Delete") && !mod && !e.isComposing) {
      const at = caret();
      const r = at && at.start === at.end ? removeBox(value, at.start, e.key === "Backspace") : null;
      if (r) {
        e.preventDefault();
        remember(r.text, r.caret, true);
        changed(r.text, { start: r.caret, end: r.caret });
        return;
      }
    }
    if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      const at = caret();
      const r = at && at.start === at.end ? (enterInItem(value, at.start) ?? enterInList(value, at.start)) : null;
      if (r) {
        remember(r.text, r.caret, true);
        changed(r.text, { start: r.caret, end: r.caret });
        return;
      }
      insertText("\n"); // a real newline in the text, never a <div> or <br> from the browser
      return;
    }
    if (!mod) return;
    const k = e.key.toLowerCase();
    if (k === "b") (e.preventDefault(), apply("**"));
    else if (k === "i") (e.preventDefault(), apply("*"));
    else if (k === "u") (e.preventDefault(), apply("__"));
    else if (k === "x" && e.shiftKey) (e.preventDefault(), apply("~~"));
    else if (k === "l" && e.shiftKey) (e.preventDefault(), checklist());
    else if (k === "z" && !e.shiftKey) (e.preventDefault(), undoStep());
    else if ((k === "z" && e.shiftKey) || k === "y") (e.preventDefault(), redoStep());
  }

  function onPaste(e: ClipboardEvent) {
    e.preventDefault();
    const files = [...(e.clipboardData?.files ?? [])];
    if (files.length && onfiles) return onfiles(files);
    insertText((e.clipboardData?.getData("text/plain") ?? "").replace(/\r\n?/g, "\n"));
  }

  // A press on a box ticks it without moving the caret there.
  function onMousedown(e: MouseEvent) {
    const box = (e.target as HTMLElement).closest<HTMLElement>(".box");
    if (!box) return;
    e.preventDefault();
    const at = caret();
    const text = toggleBox(value, Number(box.dataset.line));
    remember(text, at?.start ?? 0, true);
    changed(text, at);
  }

  function onClick(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest<HTMLElement>("[data-href]");
    if (a && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      onopenlink?.(a.dataset.href!);
    }
  }
</script>

<!-- The link card is a pointer extra; keyboard users have the right-click menu's Open / Copy link. -->
<!-- svelte-ignore a11y_no_static_element_interactions, a11y_mouse_events_have_key_events -->
<div
  class="rich"
  bind:this={el}
  contenteditable="true"
  role="textbox"
  aria-multiline="true"
  tabindex="0"
  spellcheck="true"
  oninput={onInput}
  onkeydown={onKeydown}
  onpaste={onPaste}
  ondragover={(e) => e.dataTransfer?.types.includes("Files") && e.preventDefault()}
  ondrop={(e) => {
    const files = [...(e.dataTransfer?.files ?? [])];
    if (files.length && onfiles) {
      e.preventDefault();
      onfiles(files);
    }
  }}
  onclick={onClick}
  onmouseover={onOver}
  onmouseout={onOut}
  onmousedown={onMousedown}
  oncompositionstart={() => (composing = true)}
  oncompositionend={() => {
    composing = false;
    onInput();
  }}
></div>

{#if linkCard}
  {@const c = linkCard}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="link-card" style:left="{c.x}px" style:top="{c.y}px" onmouseleave={() => (linkCard = null)}
    onmouseenter={() => clearTimeout(cardTimer)} onmousedown={(e) => e.preventDefault()}>
    <span class="site" title={c.href}>{site(c.href)}</span>
    <button onclick={() => { onopenlink?.(c.href); linkCard = null; }}>Open</button>
    <button onclick={() => copyLink(c.href)}>{copied ? "Copied" : "Copy"}</button>
  </div>
{/if}

<style>
  .link-card {
    position: fixed;
    z-index: 85;
    display: flex;
    align-items: center;
    gap: 4px;
    max-width: 240px;
    padding: 4px 4px 4px 10px;
    border-radius: 10px;
    background: var(--surface);
    border: 1px solid var(--line);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.16);
    font-size: 12px;
  }
  .link-card .site {
    flex: 1;
    min-width: 0;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .link-card button {
    all: unset;
    cursor: pointer;
    padding: 4px 9px;
    border-radius: 7px;
    color: var(--text);
    background: var(--surface-2);
  }
  .link-card button:hover {
    background: color-mix(in srgb, var(--accent) 18%, var(--surface-2));
  }
  .rich {
    white-space: pre-wrap;
    word-break: break-word;
    outline: none;
    cursor: text;
  }
  .rich :global(.b) {
    font-weight: 700;
  }
  .rich :global(.i) {
    font-style: italic;
  }
  .rich :global(.u) {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .rich :global(.s) {
    text-decoration: line-through;
    color: var(--muted);
  }
  .rich :global(.u.s) {
    text-decoration: underline line-through;
  }
  /* Markers are hidden except on the line being edited, where they show faintly. font-size 0
     (not display none) keeps them in the text, so caret positions stay exact. */
  .rich :global(.m) {
    font-size: 0;
    color: var(--faint);
    font-weight: 400;
    font-style: normal;
    text-decoration: none;
  }
  .rich:focus :global(.line.cur .m) {
    font-size: inherit;
    opacity: 0.55;
  }
  /* Checklist: the "- [ ] " text is hidden (size 0) and a box is drawn in its place. */
  .rich :global(.box) {
    font-size: 0;
    color: transparent;
    cursor: pointer;
    user-select: none;
  }
  .rich :global(.box)::before {
    content: "";
    display: inline-block;
    width: 15px;
    height: 15px;
    margin: 0 4px 0 1px;
    vertical-align: -2px;
    border: 1.5px solid var(--faint);
    border-radius: 4px;
    box-sizing: border-box;
  }
  .rich :global(.item.done .box)::before {
    background: var(--accent)
      url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Cpath d='M4 8.2l2.6 2.6L12 5.4' fill='none' stroke='white' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E")
      center / 13px no-repeat;
    border-color: var(--accent);
  }
  .rich :global(.item.done .txt) {
    text-decoration: line-through;
    color: var(--muted);
  }
  /* Links: accent text with a soft chip behind, so a pasted URL reads as a link, not noise. */
  .rich :global(.lnk) {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 9%, transparent);
    border-radius: 5px;
    padding: 1px 3px;
    text-decoration: none;
    cursor: pointer;
    word-break: break-all;
  }
  .rich :global(.lnk:hover) {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .rich :global(.lnk-more) {
    font-size: 0;
  }
  .rich :global(.lnk:has(.lnk-more))::after {
    content: "…";
  }
  .rich:focus :global(.line.cur .lnk-more) {
    font-size: inherit;
  }
  .rich:focus :global(.line.cur .lnk:has(.lnk-more))::after {
    content: none;
  }
</style>
