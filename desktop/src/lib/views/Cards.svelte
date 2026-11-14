<!-- A · Clean cards: big capture box, pinned row, cards with two-line previews. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { ago, preview, progress, type Note, type Section } from "$lib/notes";

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
</script>

{@render header()}
{@render capture("big")}
<div class="scroll">
  {#each sections as s (s.name)}
    <div class="sec">{s.name}</div>
    {#if s.name === "Pinned"}
      <div class="pins">
        {#each s.notes as n (n.id)}
          {@const p = progress(n)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="pin" class:sel={n.id === selectedId} data-id={n.id} style:--c={n.color ?? "var(--faint)"}
            onclick={() => onOpen(n)} onmouseenter={() => onHover(n)}>
            <div class="pin-t">{n.title || "Untitled"}</div>
            <div class="pin-p">{preview(n)}</div>
            <div class="pin-f">
              {#if p}<span class="bar"><i style:width="{(p.done / p.total) * 100}%"></i></span><span>{p.done}/{p.total}</span>{/if}
              <span class="time">{ago(n.updated_at)}</span>
            </div>
          </div>
        {/each}
      </div>
    {:else}
      {#each s.notes as n (n.id)}
        {@const p = progress(n)}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="card" class:sel={n.id === selectedId} data-id={n.id} onclick={() => onOpen(n)} onmouseenter={() => onHover(n)}>
          <div class="row">
            <div class="t">{n.title || "Untitled"}</div>
            <button class="pinbtn" onclick={(e) => { e.stopPropagation(); onPin(n); }} aria-label="Pin" title="Pin to the edge"><svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 17v5"/><path d="M9 3h6l-1 7 4 3v2H6v-2l4-3z"/></svg></button>
            <div class="time">{ago(n.updated_at)}</div>
          </div>
          {#if preview(n)}<div class="p">{preview(n)}</div>{/if}
          {#if p}<div class="prog"><span class="bar"><i style:width="{(p.done / p.total) * 100}%"></i></span><span>{p.done}/{p.total}</span></div>{/if}
        </div>
      {/each}
    {/if}
  {:else}
    <div class="empty">{query ? "No matches" : "Nothing here yet. Write your first thought above."}</div>
  {/each}
</div>

<style>
  .scroll { flex: 1; min-height: 0; overflow: auto; padding: 0 2px 8px; }
  .sec { font-size: 11.5px; font-weight: 600; text-transform: uppercase; letter-spacing: .07em; color: var(--faint); margin: 14px 2px 8px; }
  .pins { display: flex; gap: 10px; overflow-x: auto; padding: 2px 2px 4px; scrollbar-width: none; }
  .pins::-webkit-scrollbar { display: none; }
  .pin { flex: 0 0 150px; background: var(--surface); border-radius: 12px; padding: 10px 12px; border-top: 4px solid var(--c);
    box-shadow: 0 1px 3px var(--shadow); cursor: pointer; }
  .pin-t, .t { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pin-t { font-size: 13.5px; }
  .pin-p { color: var(--muted); font-size: 12px; height: 34px; overflow: hidden; margin-top: 2px; }
  .pin-f, .prog { display: flex; align-items: center; gap: 6px; margin-top: 6px; color: var(--faint); font-size: 11px; }
  .pin-f .time { margin-left: auto; }
  .card { background: var(--surface); border-radius: 12px; padding: 11px 14px; margin-bottom: 8px; box-shadow: 0 1px 2px var(--shadow);
    cursor: pointer; transition: box-shadow 0.12s; }
  .card.sel, .pin.sel { box-shadow: 0 0 0 1.5px color-mix(in srgb, var(--accent) 45%, transparent), 0 4px 14px var(--shadow); }
  .row { display: flex; gap: 8px; align-items: baseline; }
  .t { flex: 1; }
  .time { color: var(--faint); font-size: 11.5px; white-space: nowrap; }
  .p { color: var(--muted); font-size: 12.5px; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .bar { display: inline-block; width: 46px; height: 4px; border-radius: 2px; background: var(--surface-2); }
  .bar i { display: block; height: 100%; border-radius: 2px; background: #22c55e; }
  .pinbtn { opacity: 0; padding: 0 2px; color: var(--muted); display: grid; align-self: center; }
  .card:hover .pinbtn, .card.sel .pinbtn { opacity: 0.45; }
  .pinbtn:hover { opacity: 1 !important; }
  .empty { color: var(--faint); padding: 24px 4px; text-align: center; }
</style>
