<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { createEdge, type Mode, type Reason } from "$lib/hover";
  import { captureHotkey, formatHotkey } from "$lib/accel";
  import { edgeTabs, nextColor, PALETTE, sections as toSections, type Note } from "$lib/notes";
  import Cards from "$lib/views/Cards.svelte";
  import Compact from "$lib/views/Compact.svelte";
  import Sticky from "$lib/views/Sticky.svelte";

  type Layout = "cards" | "compact" | "sticky";
  type Settings = {
    hotkey: string;
    side: "left" | "right";
    barPosition: number;
    panelWidth: number | null;
    hoverOpen: boolean;
    showBar: boolean;
    theme: "system" | "light" | "dark";
    layout: Layout;
    barColor: string | null;
  };
  type HotkeyStatus = { accelerator: string; bound: string | null; portal: boolean; error: string | null };

  // ---- state
  let mode: Mode = $state("collapsed");
  let all: Note[] = $state([]); // every visible note, for the edge tabs
  let notes: Note[] = $state([]); // what the list shows (search results while searching)
  let query = $state("");
  let selectedId: string | null = $state(null);
  let editing: Note | null = $state(null);
  let draft = $state("");
  let quick = $state("");
  let searchEl: HTMLInputElement | undefined = $state();
  let editorEl: HTMLTextAreaElement | undefined = $state();
  let quickEl: HTMLTextAreaElement | undefined = $state();
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let focusOnHover = false;
  let os: "macos" | "windows" | "linux" = $state("linux");
  let settingsOpen = $state(false);
  let hotkey: HotkeyStatus | null = $state(null);
  let capturing = $state(false);
  let hotkeyMsg = $state("");
  let cfg: Settings = $state({
    hotkey: "",
    side: "right",
    barPosition: 0.5,
    panelWidth: null,
    hoverOpen: true,
    showBar: true,
    theme: "system",
    layout: "cards",
    barColor: null,
  });
  let autostart: boolean | null = $state(null);
  let settingsMsg = $state("");
  let dragging = false;
  /** Pinned-note tab under the pointer while collapsed: its title label shows next to it. */
  let hoverTab: string | null = $state(null);
  /** Note to open once the panel has expanded (a tab was clicked). */
  let pendingOpen: Note | null = null;

  const sections = $derived(toSections(notes, query.trim() !== ""));
  const flat = $derived(sections.flatMap((s) => s.notes));
  const edgeItems = $derived(edgeTabs(all));
  const tabCount = $derived(edgeItems.tabs.length + (edgeItems.more ? 1 : 0));
  // Keep in step with edge.rs: BAR_HEIGHT 120, TAB_HEIGHT 36, TAB_GAP 6.
  const stripHeight = $derived(120 + tabCount * 42);
  const views = { cards: Cards, compact: Compact, sticky: Sticky };
  const View = $derived(views[cfg.layout] ?? Cards);

  const edge = createEdge({
    hoverDelay: 0,
    leaveDelay: 150,
    onChange: (m: Mode, reason: Reason) => void changeMode(m, reason),
  });

  // ---- open / close (see git history for why each step is ordered this way)
  const OPEN_MS = 150;
  const CLOSE_MS = 110;
  const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, reducedMotion ? 0 : ms));
  const nextFrame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  let shown = $state(false);
  let seq = 0;
  let fixedWindow = false;
  let windowQueue: Promise<unknown> = Promise.resolve();
  const setWindow = (expanded: boolean, focus: boolean) =>
    (windowQueue = windowQueue.then(() => invoke("set_expanded", { expanded, focus })).catch((e) => log(`${e}`)));

  async function changeMode(m: Mode, reason: Reason) {
    const mine = ++seq;
    mode = m;
    hoverTab = null;
    if (m === "expanded") {
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
      const target = pendingOpen;
      pendingOpen = null;
      if (target) {
        settingsOpen = false;
        void open(target);
      } else if (reason === "hotkey") {
        editing = null;
        settingsOpen = false;
        await tick();
        quickEl?.focus();
      } else if (!editing) {
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

  // ---- notes
  async function refresh() {
    all = await invoke<Note[]>("notes_list");
    notes = query.trim() ? await invoke<Note[]>("notes_search", { query }) : all;
    if (!flat.some((n) => n.id === selectedId)) selectedId = flat[0]?.id ?? null;
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
    let updated = await invoke<Note>("note_set_pinned", { id: n.id, pinned: !n.pinned });
    if (updated.pinned && !updated.color) {
      updated = await invoke<Note>("note_set_color", { id: n.id, color: nextColor(all) });
    }
    if (editing?.id === n.id) editing = updated;
    await refresh();
  }

  async function setColor(n: Note, color: string) {
    const updated = await invoke<Note>("note_set_color", { id: n.id, color });
    if (editing?.id === n.id) editing = updated;
    await refresh();
  }

  async function back() {
    await flushSave();
    editing = null;
    await refresh();
    await tick();
    searchEl?.focus();
  }

  function select(id: string | null) {
    selectedId = id;
    void tick().then(() => document.querySelector(`[data-id="${id}"]`)?.scrollIntoView({ block: "nearest" }));
  }

  // ---- edge tabs: tell Rust how tall the strip is, and route pointer events by what is under them
  $effect(() => {
    void invoke("set_tab_count", { count: tabCount }).catch(() => {});
  });

  type EdgeTarget = { kind: "bar" } | { kind: "tab"; id: string };
  function targetAt(x: number, y: number): EdgeTarget | null {
    const el = (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>("[data-edge]");
    if (!el) return null;
    return el.dataset.edge === "tab" ? { kind: "tab", id: el.dataset.id! } : { kind: "bar" };
  }

  /** While collapsed only the main bar (and +N) opens on hover; a tab shows its title. */
  function pointerAt(x: number, y: number, entering: boolean) {
    if (mode === "expanded") {
      if (entering) edge.pointerEnter();
      else edge.pointerMove();
      return;
    }
    const t = targetAt(x, y);
    hoverTab = t?.kind === "tab" ? t.id : null;
    if (t?.kind === "bar") {
      if (entering) edge.pointerEnter();
      else edge.pointerMove();
    }
  }

  function openTab(n: Note) {
    pendingOpen = n;
    edge.click();
  }

  // ---- settings
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
    // Explicit token sets per theme: WebKitGTK resolves light-dark() inside custom properties from
    // the system preference and ignores color-scheme, so "Light" stayed dark on a dark desktop.
    document.documentElement.dataset.theme = cfg.theme;
    document.documentElement.style.colorScheme = cfg.theme === "system" ? "light dark" : cfg.theme;
  });
  $effect(() => {
    edge.setPaused(!cfg.hoverOpen);
  });

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

  // Drag the panel's inner edge to resize it (screenX deltas survive the window moving underneath).
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
      if (ev.clientX < 0 || ev.clientX > window.innerWidth) edge.pointerLeave();
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  }

  // ---- keyboard
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
    if (editing || settingsOpen || document.activeElement === quickEl) return;
    const i = flat.findIndex((n) => n.id === selectedId);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = e.key === "ArrowDown" ? Math.min(i + 1, flat.length - 1) : Math.max(i - 1, 0);
      select(flat[next]?.id ?? null);
    } else if (e.key === "Enter" && flat[i]) {
      e.preventDefault();
      void open(flat[i]);
    }
  }

  const log = (msg: string) => void invoke("debug_log", { msg });

  onMount(() => {
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
    // WebKitGTK under XWayland drops DOM mouseleave; GTK's own crossings (with x/y) are reliable.
    void listen<{ kind: "enter" | "leave"; x: number; y: number }>("edge-pointer", (e) => {
      if (dragging) return;
      if (e.payload.kind === "leave") {
        hoverTab = null;
        edge.pointerLeave();
      } else {
        pointerAt(e.payload.x, e.payload.y, true);
      }
    });
    document.addEventListener("mouseout", (e) => {
      if (!e.relatedTarget && !dragging) {
        hoverTab = null;
        edge.pointerLeave();
      }
    });
    window.addEventListener("blur", () => {
      if (edge.mode === "expanded") edge.escape();
    });
    void refresh();
  });
</script>

<svelte:window onkeydown={onKey} />

{#snippet header()}
  <header class="top">
    <div class="search">
      <span class="icon"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg></span>
      <input
        bind:this={searchEl}
        bind:value={query}
        oninput={() => void refresh()}
        placeholder="Search {all.length} notes"
        aria-label="Search notes"
      />
    </div>
    <button class="ib" onclick={openSettings} aria-label="Settings" title="Settings"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg></button>
    <button class="ib" onclick={() => edge.escape()} aria-label="Close panel" title="Close"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
  </header>
{/snippet}

{#snippet capture(variant: "big" | "slim" | "tile")}
  <div class="capture {variant}">
    <textarea
      bind:this={quickEl}
      bind:value={quick}
      rows={variant === "slim" ? 1 : variant === "big" ? 2 : 3}
      onkeydown={(e) => {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          void addQuick();
        }
      }}
      placeholder={variant === "slim" ? "+ New note" : variant === "tile" ? "+ New note" : "Write a thought…"}
      aria-label="New note"
    ></textarea>
    {#if variant !== "slim"}
      <div class="hint"><span><kbd>↵</kbd> save <kbd>⇧↵</kbd> new line</span><span>- [ ] for a checklist</span></div>
    {/if}
  </div>
{/snippet}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<main
  class:left={cfg.side === "left"}
  style:--accent={cfg.barColor ?? "#3b82f6"}
  style:--bar={cfg.barColor ?? "#8a8f99"}
  onmouseenter={(e) => pointerAt(e.clientX, e.clientY, true)}
  onmouseleave={() => {
    hoverTab = null;
    edge.pointerLeave();
  }}
  onmousemove={(e) => pointerAt(e.clientX, e.clientY, false)}
>
  <div
    class="strip"
    class:hidden={shown || !cfg.showBar}
    style:height="min({stripHeight}px, 100vh)"
    style:top="calc((100vh - min({stripHeight}px, 100vh)) * {cfg.barPosition})"
  >
    {#each edgeItems.tabs as n (n.id)}
      <button class="tab" class:hover={hoverTab === n.id} data-edge="tab" data-id={n.id} style:--c={n.color ?? "var(--bar)"}
        onclick={() => openTab(n)} aria-label="Open {n.title}" title={n.title}>
        {#if hoverTab === n.id}<span class="lbl">{n.title || "Untitled"}</span>{/if}
      </button>
    {/each}
    {#if edgeItems.more}
      <button class="tab more" data-edge="bar" onclick={() => edge.click()} aria-label="{edgeItems.more} more pinned notes">
        <span>+{edgeItems.more}</span>
      </button>
    {/if}
    <button class="bar" data-edge="bar" onclick={() => edge.click()} aria-label="Open notes ({all.length})">
      {#if all.length}<span>{all.length}</span>{/if}
    </button>
  </div>

  <div class="panel" class:shown style:--open-ms="{OPEN_MS}ms" style:--close-ms="{CLOSE_MS}ms">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize" onpointerdown={startResize} title="Drag to resize"></div>

    {#if settingsOpen}
      <header class="top">
        <button class="ib" onclick={() => (settingsOpen = false)} aria-label="Back to notes"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 18-6-6 6-6"/></svg></button>
        <h1 class="grow">Settings</h1>
        <button class="ib" onclick={() => edge.escape()} aria-label="Close panel"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
      </header>
      <div class="settings">
        <div class="group">Look</div>
        <section class="setting">
          <div class="grow">Layout</div>
          <div class="seg" role="group" aria-label="Layout">
            {#each [["cards", "Cards"], ["compact", "Compact"], ["sticky", "Sticky"]] as const as [v, label]}
              <button class:on={cfg.layout === v} onclick={() => saveSettings({ layout: v })}>{label}</button>
            {/each}
          </div>
        </section>
        <section class="setting">
          <div class="grow">Theme</div>
          <div class="seg" role="group" aria-label="Theme">
            {#each [["system", "System"], ["light", "Light"], ["dark", "Dark"]] as const as [v, label]}
              <button class:on={cfg.theme === v} onclick={() => saveSettings({ theme: v })}>{label}</button>
            {/each}
          </div>
        </section>
        <section class="setting col">
          <div>Bar colour</div>
          <div class="swatches">
            <button class="sw default" class:on={!cfg.barColor} onclick={() => saveSettings({ barColor: null })} aria-label="Default grey"></button>
            {#each PALETTE as c}
              <button class="sw" class:on={cfg.barColor === c} style:--c={c} onclick={() => saveSettings({ barColor: c })} aria-label="Colour {c}"></button>
            {/each}
          </div>
        </section>

        <div class="group">Edge</div>
        <section class="setting">
          <div class="grow">Screen edge</div>
          <div class="seg" role="group" aria-label="Screen edge">
            <button class:on={cfg.side === "left"} onclick={() => saveSettings({ side: "left" })}>Left</button>
            <button class:on={cfg.side === "right"} onclick={() => saveSettings({ side: "right" })}>Right</button>
          </div>
        </section>
        <section class="setting">
          <label class="grow" for="barpos">Bar position</label>
          <input id="barpos" type="range" min="0" max="1" step="0.01" value={cfg.barPosition}
            onchange={(e) => saveSettings({ barPosition: +e.currentTarget.value })} />
        </section>
        <section class="setting">
          <div class="grow"><div>Open on hover</div><div class="sub">Off: opens only on click or the hotkey.</div></div>
          <input type="checkbox" checked={cfg.hoverOpen} onchange={(e) => saveSettings({ hoverOpen: e.currentTarget.checked })} aria-label="Open on hover" />
        </section>
        <section class="setting">
          <div class="grow"><div>Show the bar</div><div class="sub">Off: nothing on screen, only the hotkey opens notes.</div></div>
          <input type="checkbox" checked={cfg.showBar} onchange={(e) => saveSettings({ showBar: e.currentTarget.checked })} aria-label="Show the bar" />
        </section>

        <div class="group">General</div>
        <section class="setting">
          <div class="grow"><div>Open hotkey</div><div class="sub">Opens this panel from any app, ready to type.</div></div>
          <button class="keys" class:capturing onclick={startCapture} aria-label="Change hotkey">
            {#if capturing}Press keys…{:else if hotkey}{formatHotkey(hotkey.accelerator, os)}{:else}…{/if}
          </button>
        </section>
        {#if hotkey?.portal && hotkey.bound}
          <p class="sub pad">Your desktop has it bound as {hotkey.bound.replace(/^Press /, "")}. Settings → Keyboard can change it too.</p>
        {/if}
        {#if hotkey?.error}<p class="error pad">{hotkey.error}</p>{/if}
        {#if hotkeyMsg}<p class="sub pad" role="status">{hotkeyMsg}</p>{/if}
        <section class="setting">
          <div class="grow">Start on login</div>
          <input type="checkbox" checked={!!autostart} disabled={autostart === null} onchange={toggleAutostart} aria-label="Start on login" />
        </section>
        {#if settingsMsg}<p class="error pad" role="status">{settingsMsg}</p>{/if}
      </div>
    {:else if editing}
      <header class="top">
        <button class="ib" onclick={back} aria-label="Back to notes"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 18-6-6 6-6"/></svg></button>
        <h1 class="grow ellipsis">{editing.title || "Untitled"}</h1>
        <button class="ib" class:pinned={editing.pinned} style:--c={editing.color ?? "var(--faint)"} onclick={() => editing && togglePin(editing)}
          aria-label={editing.pinned ? "Unpin" : "Pin to the edge"} title={editing.pinned ? "Unpin" : "Pin to the edge"}><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 17v5"/><path d="M9 3h6l-1 7 4 3v2H6v-2l4-3z"/></svg></button>
        <button class="ib" onclick={() => edge.escape()} aria-label="Close panel"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
      </header>
      {#if editing.pinned}
        <div class="swatches tabcolor" aria-label="Tab colour">
          {#each PALETTE as c}
            <button class="sw small" class:on={editing.color === c} style:--c={c} onclick={() => editing && setColor(editing, c)} aria-label="Tab colour {c}"></button>
          {/each}
        </div>
      {/if}
      <textarea class="editor" bind:this={editorEl} bind:value={draft} oninput={scheduleSave} spellcheck="true"></textarea>
    {:else}
      <View
        {sections}
        {selectedId}
        {query}
        {header}
        {capture}
        onOpen={(n: Note) => open(n)}
        onPin={(n: Note) => togglePin(n)}
        onHover={(n: Note) => (selectedId = n.id)}
      />
    {/if}
  </div>
</main>

<style>
  :global(:root) {
    --bg: #faf9f7;
    --surface: #ffffff;
    --surface-2: #f0eee9;
    --text: #23252b;
    --muted: #6f727a;
    --faint: #a0a3aa;
    --line: #e6e2dc;
    --shadow: rgba(0, 0, 0, 0.07);
  }
  :global(:root[data-theme="dark"]) {
    --bg: #16171b;
    --surface: #1f2126;
    --surface-2: #26292f;
    --text: #e6e7ea;
    --muted: #a2a6af;
    --faint: #6c707a;
    --line: #33363d;
    --shadow: rgba(0, 0, 0, 0.45);
  }
  @media (prefers-color-scheme: dark) {
    :global(:root[data-theme="system"]) {
      --bg: #16171b;
      --surface: #1f2126;
      --surface-2: #26292f;
      --text: #e6e7ea;
      --muted: #a2a6af;
      --faint: #6c707a;
      --line: #33363d;
      --shadow: rgba(0, 0, 0, 0.45);
    }
  }
  :global(html) {
    color-scheme: light dark; /* replaced by the Theme setting; body must inherit, not set its own */
  }
  :global(html, body) {
    margin: 0;
    height: 100%;
    overflow: hidden;
    font: 14px/1.45 "Inter", system-ui, -apple-system, "Segoe UI", sans-serif;
    background: transparent;
    color: var(--text);
    -webkit-font-smoothing: antialiased;
  }
  :global(button) {
    font: inherit;
    color: inherit;
    background: none;
    border: 0;
    cursor: pointer;
  }
  main {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }

  /* ---- the edge strip: tabs above the main bar, positioned like edge.rs */
  .strip {
    position: fixed;
    right: 0;
    width: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    transition: opacity 0.1s;
  }
  .strip.hidden {
    opacity: 0;
    pointer-events: none;
  }
  .tab,
  .bar {
    position: relative;
    width: 8px;
    padding: 0;
    border-radius: 5px 0 0 5px;
    flex: none;
  }
  .tab {
    height: 36px;
    background: var(--c);
  }
  .tab.more {
    background: var(--bar);
    opacity: 0.7;
  }
  .tab.more span,
  .bar span {
    writing-mode: vertical-rl;
    font-size: 9px;
    font-weight: 700;
    color: #fff;
  }
  .bar {
    flex: 1;
    min-height: 40px;
    background: var(--bar);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .lbl {
    position: absolute;
    right: 16px;
    top: 50%;
    transform: translateY(-50%);
    white-space: nowrap;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    background: var(--c);
    color: #fff;
    font-weight: 600;
    font-size: 12.5px;
    padding: 5px 10px;
    border-radius: 8px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
    pointer-events: none;
    animation: lbl 0.12s ease-out;
  }
  @keyframes lbl {
    from {
      opacity: 0;
      transform: translate(6px, -50%);
    }
  }
  .left .strip {
    right: auto;
    left: 0;
  }
  .left .tab,
  .left .bar {
    border-radius: 0 5px 5px 0;
  }
  .left .lbl {
    right: auto;
    left: 16px;
  }

  /* ---- the panel */
  .panel {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 14px 12px;
    box-sizing: border-box;
    background: var(--bg);
    border-radius: 16px 0 0 16px;
    box-shadow: -10px 0 30px rgba(0, 0, 0, 0.18);
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
  .left .panel {
    border-radius: 0 16px 16px 0;
    transform: translateX(-100%);
    box-shadow: 10px 0 30px rgba(0, 0, 0, 0.18);
  }
  .left .panel.shown {
    transform: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .panel,
    .panel.shown {
      transition: none;
    }
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

  /* ---- shared chrome */
  :global(.top) {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  h1 {
    font-size: 15px;
    font-weight: 600;
    margin: 0;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .ellipsis {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ib {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    display: grid;
    place-items: center;
    color: var(--muted);
    font-size: 15px;
    flex: none;
  }
  .ib:hover {
    background: var(--surface-2);
  }
  .ib.pinned {
    color: var(--c);
    background: color-mix(in srgb, var(--c) 15%, transparent);
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--surface-2);
    border-radius: 10px;
    padding: 0 10px;
  }
  .search .icon {
    color: var(--faint);
    display: grid;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: none;
    font: inherit;
    color: inherit;
    padding: 8px 0;
    outline: none;
  }
  .capture {
    background: var(--surface);
    border-radius: 12px;
    padding: 10px 12px 8px;
    box-shadow: 0 0 0 1px var(--line);
    transition: box-shadow 0.12s;
  }
  .capture:focus-within {
    box-shadow: 0 0 0 1.5px var(--accent);
  }
  .capture textarea {
    width: 100%;
    border: 0;
    background: none;
    resize: none;
    font: inherit;
    font-size: 14.5px;
    color: inherit;
    outline: none;
    padding: 0;
  }
  .capture.slim {
    padding: 8px 12px;
  }
  .capture.tile {
    flex: 1;
    background: transparent;
    box-shadow: none;
    padding: 2px;
  }
  .hint {
    display: flex;
    justify-content: space-between;
    color: var(--faint);
    font-size: 11px;
    margin-top: 4px;
    gap: 8px;
  }
  .capture.tile .hint span:last-child {
    display: none;
  }
  kbd {
    border: 1px solid var(--line);
    border-bottom-width: 2px;
    border-radius: 4px;
    padding: 0 4px;
    font: inherit;
    font-size: 10px;
  }
  .editor {
    flex: 1;
    resize: none;
    font: inherit;
    font-size: 14.5px;
    line-height: 1.6;
    color: inherit;
    background: var(--surface);
    border: 0;
    border-radius: 12px;
    padding: 14px;
    outline: none;
    box-shadow: 0 0 0 1px var(--line);
  }
  .editor:focus {
    box-shadow: 0 0 0 1.5px var(--accent);
  }

  /* ---- settings */
  .settings {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .group {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--faint);
    margin: 14px 4px 4px;
  }
  .setting {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 4px;
    border-bottom: 1px solid var(--line);
  }
  .setting.col {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
  .sub {
    color: var(--faint);
    font-size: 12px;
  }
  .pad {
    margin: 6px 4px;
  }
  .error {
    color: #e5484d;
    font-size: 12px;
  }
  .seg {
    display: flex;
    background: var(--surface-2);
    border-radius: 8px;
    padding: 2px;
  }
  .seg button {
    padding: 4px 10px;
    border-radius: 6px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .seg button.on {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px var(--shadow);
  }
  .swatches {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .tabcolor {
    margin: -2px 2px 0;
  }
  .sw {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    padding: 0;
    background: var(--c);
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.1);
  }
  .sw.small {
    width: 16px;
    height: 16px;
  }
  .sw.default {
    background: #8a8f99;
  }
  .sw.on {
    box-shadow: 0 0 0 2px var(--bg), 0 0 0 4px var(--c, #8a8f99);
  }
  .keys {
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 5px 10px;
    font-family: ui-monospace, monospace;
    font-size: 12.5px;
    white-space: nowrap;
  }
  .keys.capturing {
    border-color: var(--accent);
  }
  input[type="checkbox"] {
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
  }
  input[type="range"] {
    accent-color: var(--accent);
  }
</style>
