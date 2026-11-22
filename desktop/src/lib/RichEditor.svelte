<!-- The note editor: plain text with live formatting. The text (with its **markers**) is the value;
     on every edit it is re-rendered as styled spans and the caret is put back by character offset.
     Markers stay visible, faintly, so text and caret positions are the same as in a textarea. -->
<script lang="ts">
  import { format, type Span } from "./format";
  import { enterInItem, removeBox, toggleBox, toggleItemLine, toggleWrap } from "./edit";

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
  function span(s: Span): string {
    const cls = [s.bold && "b", s.italic && "i", s.underline && "u", s.strike && "s", s.marker && "m", s.href && "lnk"].filter(Boolean).join(" ");
    const attr = s.href ? ` data-href="${esc(s.href)}" title="Ctrl+click to open"` : "";
    return cls ? `<span class="${cls}"${attr}>${esc(s.text)}</span>` : esc(s.text);
  }
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
  function onInput() {
    if (composing) return;
    const at = caret();
    const text = read();
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
      const r = at && at.start === at.end ? enterInItem(value, at.start) : null;
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

<!-- svelte-ignore a11y_no_static_element_interactions -->
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
  onmousedown={onMousedown}
  oncompositionstart={() => (composing = true)}
  oncompositionend={() => {
    composing = false;
    onInput();
  }}
></div>

<style>
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
  .rich :global(.lnk) {
    color: var(--accent);
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--accent) 45%, transparent);
    text-underline-offset: 3px;
  }
</style>
