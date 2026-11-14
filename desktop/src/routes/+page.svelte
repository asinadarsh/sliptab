<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { createEdge, type Mode, type Reason } from "$lib/hover";
  import { captureHotkey, formatHotkey } from "$lib/accel";

  type Settings = {
    hotkey: string;
    side: "left" | "right";
    barPosition: number;
    panelWidth: number | null;
    hoverOpen: boolean;
    showBar: boolean;
    theme: "system" | "light" | "dark";
  };
  type HotkeyStatus = { accelerator: string; bound: string | null; portal: boolean; error: string | null };

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
  let os: "macos" | "windows" | "linux" = $state("linux");
  let settingsOpen = $state(false);
  let hotkey: HotkeyStatus | null = $state(null);
  let capturing = $state(false);
  let hotkeyMsg = $state("");
  let quickEl: HTMLInputElement | undefined = $state();
  let cfg: Settings = $state({
    hotkey: "",
    side: "right",
    barPosition: 0.5,
    panelWidth: null,
    hoverOpen: true,
    showBar: true,
    theme: "system",
  });
  let autostart: boolean | null = $state(null);
  let settingsMsg = $state("");
  let dragging = false;

  async function saveSettings(patch: Partial<Settings>) {
    try {
      cfg = await invoke<Settings>("save_settings", { new: { ...cfg, ...patch } });
      settingsMsg = "";
    } catch (e) {
      settingsMsg = `Could not save: ${e}`;
    }
  }

  async function toggleAutostart() {
    try {
      autostart = await invoke<boolean>("set_autostart", { enabled: !autostart });
    } catch (e) {
      settingsMsg = `Start on login: ${e}`;
    }
  }

  $effect(() => {
    document.documentElement.style.colorScheme = cfg.theme === "system" ? "light dark" : cfg.theme;
  });
  $effect(() => {
    edge.setPaused(!cfg.hoverOpen);
  });

  // Drag the panel's inner edge to resize it. screenX deltas stay correct while the window itself
  // moves under the pointer. Width updates go out at most once per frame and are saved on release.
  function startResize(e: PointerEvent) {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    dragging = true;
    const startX = e.screenX;
    const startW = window.innerWidth;
    const dir = cfg.side === "right" ? -1 : 1;
    let width = startW;
    let frame = 0;
    const move = (ev: PointerEvent) => {
      width = Math.max(320, startW + dir * (ev.screenX - startX));
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          void invoke("set_panel_width", { width, save: false });
        });
    };
    const up = (ev: PointerEvent) => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      cancelAnimationFrame(frame);
      dragging = false;
      cfg.panelWidth = width;
      void invoke("set_panel_width", { width, save: true });
      // Released outside the panel: treat it as leaving.
      if (ev.clientX < 0 || ev.clientX > window.innerWidth) edge.pointerLeave();
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  }

  const edge = createEdge({
    hoverDelay: 0,
    leaveDelay: 150,
    onChange: (m: Mode, reason: Reason) => void changeMode(m, reason),
  });

  // Slide timings, kept in step with the CSS below.
  const OPEN_MS = 150;
  const CLOSE_MS = 110;
  const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, reducedMotion ? 0 : ms));
  const nextFrame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  // The panel is always in the page, parked off-screen; opening only flips `shown`, so the slide
  // starts on the very next frame. On X11 the window never resizes, so the slide does not wait for
  // Rust either. Elsewhere the window must grow first. A newer mode change cancels an older one.
  let shown = $state(false);
  let seq = 0;
  let fixedWindow = false;
  // Window changes are queued so a late "collapse" can never land after a newer "expand".
  let windowQueue: Promise<unknown> = Promise.resolve();
  const setWindow = (expanded: boolean, focus: boolean) =>
    (windowQueue = windowQueue.then(() => invoke("set_expanded", { expanded, focus })).catch((e) => log(`${e}`)));

  async function changeMode(m: Mode, reason: Reason) {
    const mine = ++seq;
    mode = m;
    if (m === "expanded") {
      // Only take keyboard focus when the user asked for the panel on purpose,
      // except on GNOME, where focus is the only reliable way to know the user moved on.
      const focus = reason === "hotkey" || reason === "click" || (reason === "hover" && focusOnHover);
      if (fixedWindow) {
        shown = true;
        void setWindow(true, focus);
      } else {
        await setWindow(true, focus);
        await nextFrame();
        if (mine !== seq) return;
        shown = true;
      }
      // The hotkey is for capturing a thought: land in Quick note. Hover/click land in search.
      if (reason === "hotkey") {
        editing = null;
        settingsOpen = false;
        await tick();
        quickEl?.focus();
      } else {
        searchEl?.focus();
      }
      void refresh();
    } else {
      shown = false;
      void flushSave();
      await sleep(CLOSE_MS);
      if (mine !== seq) return;
      await setWindow(false, false);
      if (mine === seq) {
        editing = null;
        settingsOpen = false;
        capturing = false;
      }
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

  async function openSettings() {
    settingsOpen = true;
    hotkeyMsg = "";
    settingsMsg = "";
    hotkey = await invoke<HotkeyStatus>("get_hotkey");
    autostart = await invoke<boolean>("get_autostart").catch(() => null);
  }

  function startCapture() {
    capturing = true;
    hotkeyMsg = "Press the new keys. Esc cancels.";
  }

  async function onCaptureKey(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      capturing = false;
      hotkeyMsg = "";
      return;
    }
    const got = captureHotkey(e);
    if (!got) return;
    if ("error" in got) {
      hotkeyMsg = got.error;
      return;
    }
    capturing = false;
    try {
      hotkey = await invoke<HotkeyStatus>("set_hotkey", { accelerator: got.accel });
      hotkeyMsg = hotkey.portal ? "GNOME may ask you to confirm the new keys." : "Saved.";
    } catch (err) {
      hotkeyMsg = `${err}`;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (capturing) {
      void onCaptureKey(e);
      return;
    }
    if (mode !== "expanded") return;
    if (e.key === "Escape") {
      e.preventDefault();
      if (editing) void back();
      else if (settingsOpen) settingsOpen = false;
      else edge.escape();
      return;
    }
    if (editing || settingsOpen || document.activeElement?.id === "quick") return;
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
    invoke<{ focusOnHover: boolean; fixedWindow: boolean; os: typeof os }>("platform").then((p) => {
      focusOnHover = p.focusOnHover;
      fixedWindow = p.fixedWindow;
      os = p.os;
    });
    void invoke<Settings>("get_settings").then((c) => (cfg = c));
    void listen("edge-hotkey", () => edge.hotkey());
    void listen<HotkeyStatus>("edge-hotkey-status", (e) => (hotkey = e.payload));
    window.addEventListener("error", (e) => log(`error: ${e.message}`));
    window.addEventListener("unhandledrejection", (e) => log(`rejection: ${e.reason}`));
    // On GNOME (XWayland) WebKitGTK does not always fire mouseleave when the pointer moves onto
    // a Wayland window, so also treat "mouseout to nowhere" and losing focus as leaving.
    const leftWindow = (e: MouseEvent) => {
      if (!e.relatedTarget && !dragging) {
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
      if (dragging) return; // the pointer leaves the window while dragging the edge outwards
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
  class:left={cfg.side === "left"}
  style:--bar-pos={cfg.barPosition}
  onmouseenter={() => { log("enter"); edge.pointerEnter(); }}
  onmouseleave={() => { log("leave"); edge.pointerLeave(); }}
  onmousemove={() => edge.pointerMove()}
>
    <button class="bar" class:hidden={shown || !cfg.showBar} onclick={() => edge.click()} aria-label="Open notes ({notes.length})">
      {#if notes.length}<span class="count">{notes.length}</span>{/if}
    </button>
  <div class="panel" class:shown style:--open-ms="{OPEN_MS}ms" style:--close-ms="{CLOSE_MS}ms">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="resize" onpointerdown={startResize} title="Drag to resize"></div>
  {#if settingsOpen}
    <header>
      <button onclick={() => (settingsOpen = false)} aria-label="Back to notes">←</button>
      <span class="grow">Settings</span>
      <button onclick={() => edge.escape()} aria-label="Close panel">✕</button>
    </header>
    <section class="setting">
      <div class="grow">
        <div>Open hotkey</div>
        <div class="muted small">Opens this panel from any app, ready to type a note.</div>
      </div>
      <button class="keys" class:capturing onclick={startCapture} aria-label="Change hotkey">
        {#if capturing}Press keys…{:else if hotkey}{formatHotkey(hotkey.accelerator, os)}{:else}…{/if}
      </button>
    </section>
    {#if hotkey?.portal && hotkey.bound}
      <p class="muted small">Your desktop has it bound as: {hotkey.bound.replace(/^Press /, "")}. You can also change it in Settings → Keyboard.</p>
    {/if}
    {#if hotkey?.error}<p class="error small">{hotkey.error}</p>{/if}
    {#if hotkeyMsg}<p class="muted small" role="status">{hotkeyMsg}</p>{/if}

    <section class="setting">
      <div class="grow">Screen edge</div>
      <div class="seg" role="group" aria-label="Screen edge">
        <button class:on={cfg.side === "left"} onclick={() => saveSettings({ side: "left" })}>Left</button>
        <button class:on={cfg.side === "right"} onclick={() => saveSettings({ side: "right" })}>Right</button>
      </div>
    </section>
    <section class="setting">
      <label class="grow" for="barpos">Bar position</label>
      <input
        id="barpos"
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={cfg.barPosition}
        onchange={(e) => saveSettings({ barPosition: +e.currentTarget.value })}
      />
    </section>
    <section class="setting">
      <div class="grow">
        <div>Open on hover</div>
        <div class="muted small">Off: the bar opens only on click or the hotkey.</div>
      </div>
      <input type="checkbox" checked={cfg.hoverOpen} onchange={(e) => saveSettings({ hoverOpen: e.currentTarget.checked })} aria-label="Open on hover" />
    </section>
    <section class="setting">
      <div class="grow">
        <div>Show the bar</div>
        <div class="muted small">Off: nothing on screen, only the hotkey opens notes.</div>
      </div>
      <input type="checkbox" checked={cfg.showBar} onchange={(e) => saveSettings({ showBar: e.currentTarget.checked })} aria-label="Show the bar" />
    </section>
    <section class="setting">
      <div class="grow">Start on login</div>
      <input type="checkbox" checked={!!autostart} disabled={autostart === null} onchange={toggleAutostart} aria-label="Start on login" />
    </section>
    <section class="setting">
      <div class="grow">Theme</div>
      <div class="seg" role="group" aria-label="Theme">
        {#each ["system", "light", "dark"] as const as t}
          <button class:on={cfg.theme === t} onclick={() => saveSettings({ theme: t })}>{t[0].toUpperCase() + t.slice(1)}</button>
        {/each}
      </div>
    </section>
    {#if settingsMsg}<p class="error small" role="status">{settingsMsg}</p>{/if}
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
      <button onclick={openSettings} aria-label="Settings">⚙</button>
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
      bind:this={quickEl}
      bind:value={quick}
      onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); void addQuick(); } }}
      placeholder="Quick note, Enter to save"
      aria-label="Quick note"
    />
  {/if}
  </div>
</main>

<style>
  :global(html, body) {
    margin: 0;
    height: 100%;
    overflow: hidden;
    font: 14px/1.4 system-ui, sans-serif;
    color-scheme: light dark;
    background: transparent;
    color: CanvasText;
  }
  main {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  .panel {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 10px;
    gap: 8px;
    box-sizing: border-box;
    background: Canvas;
    border-radius: 12px 0 0 12px;
    transform: translateX(100%);
    opacity: 0.6;
    will-change: transform;
    transition:
      transform var(--close-ms) cubic-bezier(0.4, 0, 1, 1),
      opacity var(--close-ms) linear;
  }
  .panel.shown {
    transform: none;
    opacity: 1;
    transition:
      transform var(--open-ms) cubic-bezier(0.16, 1, 0.3, 1),
      opacity var(--open-ms) linear;
  }
  @media (prefers-reduced-motion: reduce) {
    .panel,
    .panel.shown {
      transition: none;
    }
  }
  .bar.hidden {
    opacity: 0;
    pointer-events: none;
  }
  /* Matches edge.rs: an 8x120 bar on the right edge, vertically centred. On X11 the window stays
     panel-sized and only this area takes the pointer; elsewhere the window is the bar itself. */
  .bar {
    position: fixed;
    right: 0;
    /* Same rule as edge.rs: bar_position 0..1 along the free space of the edge. */
    top: calc((100vh - min(120px, 100vh)) * var(--bar-pos, 0.5));
    width: 8px;
    height: min(120px, 100vh);
    border: 0;
    padding: 0;
    cursor: pointer;
    background: color-mix(in srgb, CanvasText 25%, Canvas);
    border-radius: 4px 0 0 4px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .left .bar {
    right: auto;
    left: 0;
    border-radius: 0 4px 4px 0;
  }
  .left .panel {
    border-radius: 0 12px 12px 0;
    transform: translateX(-100%);
  }
  .left .panel.shown {
    transform: none;
  }
  .panel {
    position: relative;
  }
  .resize {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 6px;
    cursor: ew-resize;
    z-index: 1;
  }
  .left .resize {
    left: auto;
    right: 0;
  }
  .seg {
    display: flex;
    border: 1px solid color-mix(in srgb, CanvasText 25%, Canvas);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    padding: 4px 10px;
  }
  .seg button.on {
    background: color-mix(in srgb, Highlight 35%, Canvas);
  }
  input[type="checkbox"] {
    width: 18px;
    height: 18px;
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
  .setting {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 4px;
  }
  .keys {
    border: 1px solid color-mix(in srgb, CanvasText 25%, Canvas);
    border-radius: 6px;
    padding: 6px 10px;
    font-family: ui-monospace, monospace;
    white-space: nowrap;
  }
  .keys.capturing {
    border-color: Highlight;
  }
  .small {
    font-size: 12px;
    margin: 0 4px;
  }
  .error {
    color: #d33;
  }
  header {
    display: flex;
    gap: 8px;
    align-items: center;
  }
</style>
