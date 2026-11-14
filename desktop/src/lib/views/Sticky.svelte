<!-- C · Sticky notes: tiles in a grid, pinned ones tinted with their tab colour. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { ago, checklistItems, preview, type Note, type Section } from "$lib/notes";

  let {
    sections,
    selectedId,
    query,
    onOpen,
    onPin,
    onHover,
    header,
    capture,
  }: {
    sections: Section[];
    selectedId: string | null;
    query: string;
    onOpen: (n: Note) => void;
    onPin: (n: Note) => void;
    onHover: (n: Note) => void;
    header: Snippet;
    capture: Snippet<["big" | "slim" | "tile"]>;
  } = $props();

  const all = $derived(sections.flatMap((s) => s.notes));
</script>

{@render header()}
<div class="grid">
  {#if !query}<div class="tile new">{@render capture("tile")}</div>{/if}
  {#each all as n (n.id)}
    {@const items = checklistItems(n)}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="tile" class:pinned={n.pinned} class:sel={n.id === selectedId} data-id={n.id}
      style:--c={n.color ?? "var(--faint)"} onclick={() => onOpen(n)} onmouseenter={() => onHover(n)}>
      <div class="tt">
        <button class="pinm" class:on={n.pinned} aria-label={n.pinned ? "Unpin" : "Pin"} title={n.pinned ? "Unpin" : "Pin to the edge"}
          onclick={(e) => { e.stopPropagation(); onPin(n); }}>●</button>{n.title || "Untitled"}
      </div>
      {#if items.length}
        {#each items.slice(0, 4) as it}<div class="ci" class:done={it.done}>{it.done ? "☑" : "☐"} {it.text}</div>{/each}
        {#if items.length > 4}<div class="ci more">+{items.length - 4} more</div>{/if}
      {:else}
        <div class="tp">{preview(n)}</div>
      {/if}
      <div class="tm">{ago(n.updated_at)}</div>
    </div>
  {/each}
  {#if query && !all.length}<div class="empty">No matches</div>{/if}
</div>

<style>
  .grid { flex: 1; min-height: 0; overflow: auto; display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px; align-content: start; padding: 2px 2px 8px; }
  .tile { border-radius: 14px; padding: 12px 12px 26px; min-height: 120px; background: var(--surface);
    box-shadow: 0 1px 3px var(--shadow); position: relative; cursor: pointer; transition: transform .12s, box-shadow .12s; }
  .tile:nth-child(3n) { transform: rotate(.4deg); }
  .tile:nth-child(4n) { transform: rotate(-.5deg); }
  .tile:hover, .tile.sel { transform: translateY(-1px); box-shadow: 0 0 0 1.5px color-mix(in srgb, var(--accent) 45%, transparent), 0 6px 16px var(--shadow); }
  .tile.pinned { background: color-mix(in srgb, var(--c) 18%, var(--surface)); }
  .tt { font-weight: 650; font-size: 13.5px; margin-bottom: 4px; overflow-wrap: anywhere; }
  .pinm { font-size: 10px; margin-right: 5px; vertical-align: 2px; padding: 0; color: var(--line); }
  .pinm.on { color: var(--c); }
  .pinm:hover { color: var(--c); }
  .tp { color: var(--muted); font-size: 12.5px; display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .ci { font-size: 12.5px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ci.done { color: var(--faint); text-decoration: line-through; }
  .ci.more { color: var(--faint); }
  .tm { position: absolute; right: 12px; bottom: 8px; font-size: 11px; color: var(--faint); }
  .new { border: 2px dashed var(--line); background: transparent; box-shadow: none; padding: 8px; cursor: text;
    display: flex; transform: none !important; }
  .empty { grid-column: 1 / -1; color: var(--faint); padding: 24px; text-align: center; }
</style>
