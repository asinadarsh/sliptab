<!-- A note's attachments: image thumbnails that open full size, audio players, and other files
     that open in their own app. Images and audio are
     fetched from the app (which downloads them from the sync server when this laptop lacks them)
     and shown from blob URLs, so the page never needs file access. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { Attachment } from "./notes";

  let { items, onremove }: { items: Attachment[]; onremove: (a: Attachment) => void } = $props();

  // sha256 -> blob URL (or an error), kept while the note is open.
  let urls: Record<string, string> = $state({});
  let failed: Record<string, string> = $state({});
  let big: Attachment | null = $state(null);

  $effect(() => {
    for (const a of items) {
      if (a.kind === "file" || urls[a.sha256] || failed[a.sha256]) continue; // files load on open
      urls[a.sha256] = ""; // loading
      invoke<ArrayBuffer>("media_get", { sha256: a.sha256 })
        .then((buf) => (urls[a.sha256] = URL.createObjectURL(new Blob([buf], { type: a.mime }))))
        .catch((e) => {
          delete urls[a.sha256];
          failed[a.sha256] = String(e);
        });
    }
  });
  $effect(() => () => Object.values(urls).forEach((u) => u && URL.revokeObjectURL(u)));

  let opening: string | null = $state(null);
  async function openFile(a: Attachment) {
    opening = a.id;
    delete failed[a.id];
    try {
      await invoke("media_open", { sha256: a.sha256, name: a.name ?? "file" });
    } catch (e) {
      failed[a.id] = String(e);
    } finally {
      opening = null;
    }
  }
  const ext = (a: Attachment) => (a.name?.includes(".") ? a.name.split(".").pop()!.slice(0, 4).toUpperCase() : "FILE");
  const size = (b: number) => (b < 1e6 ? `${Math.max(1, Math.round(b / 1e3))} KB` : `${(b / 1e6).toFixed(1)} MB`);
  const length = (ms?: number | null) => (ms ? `${Math.floor(ms / 60000)}:${String(Math.round(ms / 1000) % 60).padStart(2, "0")}` : "");
</script>

{#if items.length}
  <div class="atts">
    {#each items as a (a.id)}
      <div class="att {a.kind}">
        {#if a.kind === "file"}
          <button class="file" onclick={() => openFile(a)} title="Open {a.name ?? 'file'}">
            <span class="ext">{ext(a)}</span>
            <span class="name">{a.name ?? "File"}</span>
            <span class="size">{opening === a.id ? "Opening…" : size(a.bytes)}</span>
          </button>
          {#if failed[a.id]}<div class="msg">{failed[a.id]}</div>{/if}
        {:else if failed[a.sha256]}
          <div class="msg" title={failed[a.sha256]}>{a.kind === "image" ? "Image" : "Audio"} not here yet</div>
        {:else if !urls[a.sha256]}
          <div class="msg">Loading…</div>
        {:else if a.kind === "image"}
          <button class="thumb" onclick={() => (big = a)} aria-label="Open image"><img src={urls[a.sha256]} alt={a.name ?? "Image"} /></button>
        {:else}
          <div class="audio">
            <span class="name">{a.name ?? "Audio"}{length(a.duration_ms) ? ` · ${length(a.duration_ms)}` : ""} · {size(a.bytes)}</span>
            <audio controls preload="metadata" src={urls[a.sha256]}></audio>
          </div>
        {/if}
        <button class="x" onclick={() => onremove(a)} aria-label="Remove" title="Remove">×</button>
      </div>
    {/each}
  </div>
{/if}

{#if big && urls[big.sha256]}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="big" onclick={() => (big = null)}>
    <img src={urls[big.sha256]} alt={big.name ?? "Image"} />
  </div>
{/if}

<style>
  .atts {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 2px 2px 8px;
  }
  .att {
    position: relative;
    border-radius: 10px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .att.image {
    width: 96px;
    height: 96px;
  }
  .att.audio,
  .att.file {
    flex: 1 1 100%;
  }
  .file {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 34px 8px 8px;
    cursor: pointer;
  }
  .file .ext {
    flex: none;
    width: 38px;
    height: 38px;
    border-radius: 8px;
    display: grid;
    place-items: center;
    font-size: 10px;
    font-weight: 700;
    color: var(--muted);
    background: var(--surface);
  }
  .file .name {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .file .size {
    font-size: 11.5px;
    color: var(--faint);
  }
  .thumb {
    all: unset;
    cursor: zoom-in;
    display: block;
    width: 100%;
    height: 100%;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .audio {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 34px 8px 10px;
  }
  .audio audio {
    width: 100%;
    height: 32px;
  }
  .name {
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .msg {
    padding: 10px;
    font-size: 11.5px;
    color: var(--faint);
  }
  .x {
    all: unset;
    position: absolute;
    top: 4px;
    right: 4px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    color: #fff;
    background: rgba(0, 0, 0, 0.5);
    opacity: 0;
  }
  .att:hover .x,
  .x:focus-visible {
    opacity: 1;
  }
  .audio + .x,
  .att.audio .x,
  .att.file .x {
    opacity: 1;
    color: var(--muted);
    background: none;
  }
  .big {
    position: fixed;
    inset: 0;
    z-index: 80;
    background: rgba(0, 0, 0, 0.82);
    display: grid;
    place-items: center;
    padding: 20px;
    cursor: zoom-out;
  }
  .big img {
    max-width: 100%;
    max-height: 100%;
    border-radius: 8px;
  }
</style>
