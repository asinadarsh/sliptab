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
  // Copy and save. `note` says what happened for a moment ("Saved to Downloads").
  let note = $state("");
  let noteTimer: ReturnType<typeof setTimeout> | undefined;
  function say(m: string) {
    note = m;
    clearTimeout(noteTimer);
    noteTimer = setTimeout(() => (note = ""), 3500);
  }
  async function save(a: Attachment) {
    try {
      const path = await invoke<string>("media_save", { sha256: a.sha256, name: a.name ?? defaultName(a) });
      say(`Saved to Downloads: ${path.split(/[\\/]/).pop()}`);
    } catch (e) {
      say(`Could not save: ${e}`);
    }
  }
  async function copyImage(a: Attachment) {
    try {
      // The page's clipboard (WebView2 on Windows) takes PNG; WebKitGTK cannot, so the app does it.
      const url = urls[a.sha256];
      if (!url || typeof ClipboardItem === "undefined") throw new Error("no page clipboard");
      const img = new Image();
      img.src = url;
      await img.decode();
      const c = document.createElement("canvas");
      [c.width, c.height] = [img.naturalWidth, img.naturalHeight];
      c.getContext("2d")!.drawImage(img, 0, 0);
      const png = await new Promise<Blob>((ok, no) => c.toBlob((b) => (b ? ok(b) : no(new Error("no png"))), "image/png"));
      await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
      say("Image copied");
    } catch {
      try {
        await invoke("media_copy_image", { sha256: a.sha256 });
        say("Image copied");
      } catch (e) {
        say(`Could not copy: ${e}`);
      }
    }
  }
  const defaultName = (a: Attachment) => (a.kind === "image" ? `image.${a.mime.split("/")[1] ?? "jpg"}` : "file");

  // Right-click on an attachment: Copy (images), Save to Downloads, Remove.
  let ctx: { a: Attachment; x: number; y: number } | null = $state(null);
  function onCtx(e: MouseEvent, a: Attachment) {
    e.preventDefault();
    e.stopPropagation();
    ctx = { a, x: e.clientX, y: e.clientY };
  }
  $effect(() => {
    if (!ctx) return;
    const close = () => (ctx = null);
    const t = setTimeout(() => window.addEventListener("pointerdown", close, { once: true }));
    return () => (clearTimeout(t), window.removeEventListener("pointerdown", close));
  });

  const ext = (a: Attachment) => (a.name?.includes(".") ? a.name.split(".").pop()!.slice(0, 4).toUpperCase() : "FILE");
  const size = (b: number) => (b < 1e6 ? `${Math.max(1, Math.round(b / 1e3))} KB` : `${(b / 1e6).toFixed(1)} MB`);
  const length = (ms?: number | null) => (ms ? `${Math.floor(ms / 60000)}:${String(Math.round(ms / 1000) % 60).padStart(2, "0")}` : "");
</script>

{#if items.length}
  <div class="atts">
    {#each items as a (a.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="att {a.kind}" oncontextmenu={(e) => onCtx(e, a)}>
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
        <div class="acts">
          {#if a.kind === "image" && urls[a.sha256]}
            <button onclick={() => copyImage(a)} aria-label="Copy image" title="Copy image"><svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg></button>
          {/if}
          <button onclick={() => save(a)} aria-label="Save to Downloads" title="Save to Downloads"><svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3v12M7 10l5 5 5-5M5 21h14"/></svg></button>
          <button onclick={() => onremove(a)} aria-label="Remove" title="Remove">×</button>
        </div>
      </div>
    {/each}
  </div>
{/if}

{#if note}<div class="said">{note}</div>{/if}

{#if ctx}
  {@const a = ctx.a}
  <div class="actx" role="menu" style:left="min({ctx.x}px, calc(100vw - 200px))" style:top="min({ctx.y}px, calc(100vh - 130px))">
    {#if a.kind === "image" && urls[a.sha256]}<button role="menuitem" onclick={() => { ctx = null; void copyImage(a); }}>Copy image</button>{/if}
    <button role="menuitem" onclick={() => { ctx = null; void save(a); }}>Save to Downloads</button>
    <button role="menuitem" class="danger" onclick={() => { ctx = null; onremove(a); }}>Remove</button>
  </div>
{/if}

{#if big && urls[big.sha256]}
  {@const b = big}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="big" onclick={() => (big = null)} oncontextmenu={(e) => onCtx(e, b)}>
    <img src={urls[b.sha256]} alt={b.name ?? "Image"} />
    <div class="bigacts" onclick={(e) => e.stopPropagation()}>
      <button onclick={() => copyImage(b)}>Copy image</button>
      <button onclick={() => save(b)}>Save to Downloads</button>
    </div>
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
  .acts {
    position: absolute;
    top: 4px;
    right: 4px;
    display: flex;
    gap: 3px;
    opacity: 0;
  }
  .att:hover .acts,
  .acts:focus-within {
    opacity: 1;
  }
  .acts button {
    all: unset;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    color: #fff;
    background: rgba(0, 0, 0, 0.55);
  }
  .att.audio .acts,
  .att.file .acts {
    opacity: 1;
    top: 50%;
    transform: translateY(-50%);
  }
  .att.audio .acts button,
  .att.file .acts button {
    color: var(--muted);
    background: none;
  }
  .att.audio .audio,
  .att.file .file {
    padding-right: 74px;
  }
  .said {
    font-size: 12px;
    color: var(--muted);
    padding: 0 2px 6px;
  }
  .actx {
    position: fixed;
    z-index: 90;
    min-width: 180px;
    padding: 5px;
    border-radius: 11px;
    background: var(--surface);
    border: 1px solid var(--line);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.18);
    display: flex;
    flex-direction: column;
  }
  .actx button {
    all: unset;
    padding: 7px 11px;
    border-radius: 7px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
  }
  .actx button:hover {
    background: var(--surface-2);
  }
  .actx .danger {
    color: #e5484d;
  }
  .bigacts {
    position: absolute;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: 8px;
    cursor: default;
  }
  .bigacts button {
    all: unset;
    padding: 7px 14px;
    border-radius: 9px;
    font-size: 13px;
    color: #fff;
    background: rgba(255, 255, 255, 0.16);
    cursor: pointer;
  }
  .bigacts button:hover {
    background: rgba(255, 255, 255, 0.26);
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
