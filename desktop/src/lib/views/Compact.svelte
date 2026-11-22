<!-- B · Compact list: dense keyboard-first rows; the selected row shows a short preview. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { pinDrag } from "$lib/pindrag";
  import { ago, attachSummary, preview, progress, type Note, type Section } from "$lib/notes";

  let {
    sections,
    selectedId,
    query,
    onOpen,
    onPin,
    onMovePin,
    unseen = new Set(),
    onHover,
    header,
    capture,
  }: {
    sections: Section[];
    selectedId: string | null;
    query: string;
    onOpen: (n: Note) => void;
    onPin: (n: Note) => void;
    /** Drag a pinned note: its tab goes before `before` (null: last). */
    onMovePin: (id: string, before: string | null) => void;
    /** Pinned on another device and not opened here yet. */
    unseen?: Set<string>;
    onHover: (n: Note) => void;
    header: Snippet;
    capture: Snippet<["big" | "slim" | "tile"]>;
  } = $props();
</script>

{@render header()}
<div class="list notes-scroll" use:pinDrag={onMovePin}>
  {#each sections as s (s.name)}
    <div class="sec">{s.name}</div>
    {#each s.notes as n (n.id)}
      {@const p = progress(n)}
      {@const sel = n.id === selectedId}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="r" class:sel data-id={n.id} data-pin={n.pinned ? n.id : undefined} onclick={() => onOpen(n)} onmouseenter={() => onHover(n)}>
        <button class="dot" class:on={n.pinned} style:--c={n.color ?? "var(--line)"} aria-label={n.pinned ? "Unpin" : "Pin"}
          title={n.pinned ? "Unpin" : "Pin to the edge"} onclick={(e) => { e.stopPropagation(); onPin(n); }}></button>
        <span class="t">{#if unseen.has(n.id)}<span class="new" style:--c={n.color ?? "var(--accent)"}>New</span>{/if}{n.title || "Untitled"}</span>
        <span class="p">{preview(n) || attachSummary(n)}</span>
        {#if p}<span class="cn">{p.done}/{p.total}</span>{/if}
        <span class="time">{ago(n.updated_at)}</span>
      </div>
      {#if sel && preview(n)}
        <div class="peek">{preview(n).slice(0, 220)}<br /><span>Edited {ago(n.updated_at)} · ↵ to open</span></div>
      {/if}
    {/each}
  {:else}
    <div class="empty">{query ? "No matches" : "No notes yet."}</div>
  {/each}
</div>
{@render capture("slim")}
<div class="keys"><span>↑↓ move</span><span>↵ open</span><span>esc close</span></div>

<style>
  .list { flex: 1; min-height: 0; overflow: auto; margin: 0 -4px; }
  .sec { font-size: 11px; font-weight: 600; color: var(--faint); padding: 12px 10px 4px; letter-spacing: .05em; text-transform: uppercase; }
  .r { display: flex; align-items: center; gap: 10px; padding: 7px 10px; border-radius: 8px; white-space: nowrap; cursor: pointer; }
  .r.sel { background: var(--surface-2); box-shadow: inset 2px 0 0 var(--accent); }
  .dot { width: 9px; height: 9px; border-radius: 50%; flex: none; padding: 0; background: var(--line); }
  .dot.on { background: var(--c); }
  .dot:hover { outline: 2px solid color-mix(in srgb, var(--c) 40%, transparent); }
  .t { font-weight: 500; flex: none; max-width: 55%; overflow: hidden; text-overflow: ellipsis; }
  .p { color: var(--faint); overflow: hidden; text-overflow: ellipsis; flex: 1; font-size: 12.5px; }
  .cn { color: #22c55e; font-size: 11px; }
  .time { color: var(--faint); font-size: 11.5px; flex: none; }
  .peek { margin: 2px 10px 6px 29px; padding: 10px 12px; border-radius: 8px; background: var(--surface); color: var(--muted);
    font-size: 12.5px; border: 1px solid var(--line); }
  .peek span { color: var(--faint); font-size: 11px; }
  .keys { padding: 8px 4px 0; color: var(--faint); font-size: 11px; display: flex; gap: 14px; }
  .empty { color: var(--faint); padding: 24px 4px; text-align: center; }
  .new { display: inline-block; font-size: 9.5px; font-weight: 700; text-transform: uppercase; color: #fff;
    background: var(--c); border-radius: 4px; padding: 1px 5px; margin-right: 6px; vertical-align: 1px; }
</style>
