/**
 * Drag pinned notes to reorder their tabs. A Svelte action on the element holding them: each
 * draggable child has `data-pin="<note id>"`. Pointer events (not HTML drag and drop, which
 * WebKitGTK on XWayland handles badly); a press that moves less than a few pixels stays a click.
 * Dropped on another card it takes that card's place. On drop it calls `onmove(id, beforeId)`, beforeId null meaning "to the end".
 */
export function pinDrag(node: HTMLElement, onmove: (id: string, before: string | null) => void) {
  let start: { id: string; x: number; y: number; el: HTMLElement } | null = null;
  let dragging = false;
  let target: { id: string; after: boolean } | null = null;

  const items = () => [...node.querySelectorAll<HTMLElement>("[data-pin]")];
  const clear = () => items().forEach((e) => e.classList.remove("drop-before", "drop-after", "dragging"));

  function down(e: PointerEvent) {
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-pin]");
    if (!el || e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    e.preventDefault(); // no text selection while dragging
    start = { id: el.dataset.pin!, x: e.clientX, y: e.clientY, el };
  }
  function move(e: PointerEvent) {
    if (!start) return;
    if (!dragging) {
      if (Math.hypot(e.clientX - start.x, e.clientY - start.y) < 6) return;
      dragging = true;
      start.el.classList.add("dragging");
      node.setPointerCapture(e.pointerId);
    }
    clear();
    start.el.classList.add("dragging");
    target = null;
    const order = items().map((e) => e.dataset.pin!);
    const from = order.indexOf(start.id);
    for (const el of items()) {
      if (el.dataset.pin === start.id) continue;
      const r = el.getBoundingClientRect();
      if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) continue;
      // Dropped on a card, the dragged one takes its place: after it when moving forward,
      // before it when moving back (so with two pins, dropping on the other swaps them).
      const after = order.indexOf(el.dataset.pin!) > from;
      target = { id: el.dataset.pin!, after };
      el.classList.add(after ? "drop-after" : "drop-before");
    }
  }
  function up() {
    if (dragging && start && target) {
      const list = items().map((e) => e.dataset.pin!).filter((id) => id !== start!.id);
      const i = list.indexOf(target.id) + (target.after ? 1 : 0);
      onmove(start.id, list[i] ?? null);
    }
    if (dragging) {
      // The click that ends a drag must not open the note.
      const stop = (e: Event) => (e.stopPropagation(), e.preventDefault());
      window.addEventListener("click", stop, { capture: true, once: true });
      setTimeout(() => window.removeEventListener("click", stop, { capture: true }), 0);
    }
    clear();
    start = null;
    dragging = false;
    target = null;
  }

  node.addEventListener("pointerdown", down);
  node.addEventListener("pointermove", move);
  node.addEventListener("pointerup", up);
  node.addEventListener("pointercancel", up);
  return {
    update(f: typeof onmove) {
      onmove = f;
    },
    destroy() {
      node.removeEventListener("pointerdown", down);
      node.removeEventListener("pointermove", move);
      node.removeEventListener("pointerup", up);
      node.removeEventListener("pointercancel", up);
    },
  };
}
