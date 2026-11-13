<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { createEdge, type Mode, type Reason } from "$lib/hover";

  type Note = {
    id: string;
    title: string;
    body: string;
    pinned: boolean;
    updated_at: number;
  };

  let mode: Mode = $state("collapsed");
  let notes: Note[] = $state([]);
  let query = $state("");
  let selected = $state(0);
  let editing: Note | null = $state(null);
  let draft = $state("");
  let quick = $state("");
  let searchEl: HTMLInputElement | undefined = $state();
  let editorEl: HTMLTextAreaElement | undefined = $state();
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let focusOnHover = false;

  const edge = createEdge({
    hoverDelay: 600,
    leaveDelay: 400,
    onChange: (m: Mode, reason: Reason) => void changeMode(m, reason),
  });

  async function changeMode(m: Mode, reason: Reason) {
    if (m === "collapsed") await flushSave();
    // Only take keyboard focus when the user asked for the panel on purpose,
    // except on GNOME, where focus is the only reliable way to know the user moved on.
    const focus = reason === "hotkey" || reason === "click" || (reason === "hover" && focusOnHover);
    await invoke("set_expanded", { expanded: m === "expanded", focus });
    mode = m;
    if (m === "expanded") {
      await refresh();
      await tick();
      searchEl?.focus();
    } else {
      editing = null;
    }
  }

  async function refresh() {
    notes = query.trim()
      ? await invoke<Note[]>("notes_search", { query })
      : await invoke<Note[]>("notes_list");
    selected = Math.min(selected, Math.max(notes.length - 1, 0));
  }

  async function open(n: Note) {
    await flushSave();
    editing = n;
    draft = n.body;
    await tick();
    editorEl?.focus();
  }

  function scheduleSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(flushSave, 500);
  }

  async function flushSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    if (editing && draft !== editing.body) {
      editing = await invoke<Note>("note_update", { id: editing.id, body: draft });
    }
  }

  async function addQuick() {
    const body = quick.trim();
    if (!body) return;
    await invoke("note_create", { body });
    quick = "";
    await refresh();
  }

  async function togglePin(n: Note) {
    await invoke("note_set_pinned", { id: n.id, pinned: !n.pinned });
    await refresh();
  }

  async function back() {
    await flushSave();
    editing = null;
    await refresh();
    await tick();
    searchEl?.focus();
  }

  function onKey(e: KeyboardEvent) {
    if (mode !== "expanded") return;
    if (e.key === "Escape") {
      e.preventDefault();
      if (editing) void back();
      else edge.escape();
      return;
    }
    if (editing || document.activeElement?.id === "quick") return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selected = Math.min(selected + 1, notes.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
    } else if (e.key === "Enter" && notes[selected]) {
      e.preventDefault();
      void open(notes[selected]);
    }
  }

  function ago(ms: number) {
    const s = Math.round((Date.now() - ms) / 1000);
    if (s < 60) return "now";
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    if (s < 86400) return `${Math.floor(s / 3600)}h`;
    return `${Math.floor(s / 86400)}d`;
  }

  const log = (msg: string) => void invoke("debug_log", { msg });

  onMount(() => {
    log("mounted");
    invoke<boolean>("focus_on_hover").then((v) => (focusOnHover = v));
    window.addEventListener("error", (e) => log(`error: ${e.message}`));
    window.addEventListener("unhandledrejection", (e) => log(`rejection: ${e.reason}`));
    // On GNOME (XWayland) WebKitGTK does not always fire mouseleave when the pointer moves onto
    // a Wayland window, so also treat "mouseout to nowhere" and losing focus as leaving.
    const leftWindow = (e: MouseEvent) => {
      if (!e.relatedTarget) {
        log("mouseout-window");
        edge.pointerLeave();
      }
    };
    const blurred = () => {
      log("blur");
      if (edge.mode === "expanded") edge.escape();
    };
    document.addEventListener("mouseout", leftWindow);
    // Native crossings from GTK (Linux), more reliable than the DOM ones under XWayland.
    void listen<string>("edge-pointer", (e) => {
      log(`native-${e.payload}`);
      if (e.payload === "leave") edge.pointerLeave();
      else edge.pointerEnter();
    });
    window.addEventListener("blur", blurred);
    void refresh();
  });
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<main
  class:expanded={mode === "expanded"}
  onmouseenter={() => { log("enter"); edge.pointerEnter(); }}
  onmouseleave={() => { log("leave"); edge.pointerLeave(); }}
  onmousemove={() => edge.pointerMove()}
>
  {#if mode === "collapsed"}
    <button class="bar" onclick={() => edge.click()} aria-label="Open notes ({notes.length})">
      {#if notes.length}<span class="count">{notes.length}</span>{/if}
    </button>
  {:else if editing}
    <header>
      <button onclick={back} aria-label="Back to notes">←</button>
      <span class="muted grow">{editing.title || "Untitled"}</span>
      <button onclick={() => edge.escape()} aria-label="Close panel">✕</button>
    </header>
    <textarea bind:this={editorEl} bind:value={draft} oninput={scheduleSave} spellcheck="true"></textarea>
  {:else}
    <header>
      <input
        class="grow"
        bind:this={searchEl}
      bind:value={query}
      oninput={() => { selected = 0; void refresh(); }}
        placeholder="Search notes"
        aria-label="Search notes"
      />
      <button onclick={() => edge.escape()} aria-label="Close panel">✕</button>
    </header>
    <ul>
      {#each notes as n, i (n.id)}
        <li class:sel={i === selected}>
          <button class="note" onclick={() => open(n)} onmouseenter={() => (selected = i)}>
            <span class="title">{n.title || "Untitled"}</span>
            <span class="muted">{ago(n.updated_at)}</span>
          </button>
          <button class="pin" class:on={n.pinned} onclick={() => togglePin(n)} aria-label={n.pinned ? "Unpin" : "Pin"}>📌</button>
        </li>
      {:else}
        <li class="muted empty">{query ? "No matches" : "No notes yet. Type one below."}</li>
      {/each}
    </ul>
    <input
      id="quick"
      bind:value={quick}
      onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); void addQuick(); } }}
      placeholder="Quick note, Enter to save"
      aria-label="Quick note"
    />
  {/if}
</main>

<style>
  :global(html, body) {
    margin: 0;
    height: 100%;
    overflow: hidden;
    font: 14px/1.4 system-ui, sans-serif;
    color-scheme: light dark;
    background: Canvas;
    color: CanvasText;
  }
  main {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  main.expanded {
    padding: 10px;
    gap: 8px;
    box-sizing: border-box;
  }
  .bar {
    flex: 1;
    border: 0;
    padding: 0;
    cursor: pointer;
    background: color-mix(in srgb, CanvasText 25%, Canvas);
    border-radius: 4px 0 0 4px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .count {
    writing-mode: vertical-rl;
    font-size: 10px;
  }
  input,
  textarea {
    font: inherit;
    color: inherit;
    background: color-mix(in srgb, CanvasText 6%, Canvas);
    border: 1px solid color-mix(in srgb, CanvasText 20%, Canvas);
    border-radius: 6px;
    padding: 8px;
  }
  textarea {
    flex: 1;
    resize: none;
  }
  ul {
    flex: 1;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    border-radius: 6px;
  }
  li.sel {
    background: color-mix(in srgb, Highlight 25%, Canvas);
  }
  button {
    font: inherit;
    color: inherit;
    background: none;
    border: 0;
    cursor: pointer;
  }
  .note {
    flex: 1;
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 8px;
    text-align: left;
    min-width: 0;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pin {
    opacity: 0.25;
  }
  .pin.on {
    opacity: 1;
  }
  .muted {
    opacity: 0.6;
  }
  .empty {
    padding: 8px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  header {
    display: flex;
    gap: 8px;
    align-items: center;
  }
</style>
