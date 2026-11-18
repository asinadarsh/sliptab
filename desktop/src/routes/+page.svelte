<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { createEdge, type Mode, type Reason } from "$lib/hover";
  import { captureHotkey, formatHotkey } from "$lib/accel";
  import { ago, checklistItems, edgeTabs, nextColor, PALETTE, sections as toSections, type Note, isOnlyLink, platformName, saveTitle, PLATFORMS } from "$lib/notes";
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
  let tab: "notes" | "saved" = $state("notes");
  let allSaves: Note[] = $state([]);
  let found: Note[] = $state([]); // saves matching the search
  let folders: [string, number][] = $state([]);
  let defaultFolders: string[] = $state([]);
  let today: Note[] = $state([]); // saves to bring back today, also the bookmark tab on the edge
  let pendingSaved = false; // the edge's bookmark tab was clicked: open on Saved
  let folder: string | null = $state(null); // the chosen folder chip; null: all, "" : unsorted
  const saves = $derived(folder === null ? found : found.filter((n) => (n.link?.folder ?? "") === folder));
  const unsorted = $derived(allSaves.filter((n) => !n.link?.folder).length);
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

  // ---- sync (runs in Rust; the page shows its status and does sign-in)
  type SyncStatus = { email: string | null; lastSyncedAt: number | null; error: string | null; received: number };
  let sync: SyncStatus = $state({ email: null, lastSyncedAt: null, error: null, received: 0 });
  let syncBusy = $state(false);
  let syncMsg = $state("");
  /** A note arrived from another device since the panel was last open: dot on the bar. */
  let arrived = $state(false);
  /** Pinned-note tab under the pointer while collapsed: its title label shows next to it. */
  let hoverTab: string | null = $state(null);
  /** Note to open once the panel has expanded (a tab was clicked). */
  let pendingOpen: Note | null = null;

  const sections = $derived(toSections(notes, query.trim() !== ""));
  const flat = $derived(sections.flatMap((s) => s.notes));
  const edgeItems = $derived(edgeTabs(all));
  const tabCount = $derived(edgeItems.tabs.length + (edgeItems.more ? 1 : 0) + (today.length ? 1 : 0));
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
  // Exits from GNOME/XWayland are reported with the LAST position seen inside the window, so the
  // position of a leave says nothing. What we can trust is the live position GTK polls while the
  // pointer is on us (`move` events). One false leave arrives right after the panel opens while
  // the pointer is still pressed against the screen edge: during a short grace period a leave is
  // dropped when the last polled position was on the outer edge, or when an enter follows.
  const GRACE_MS = 400;
  let openedAt = 0;
  let lastMove: { x: number; y: number } | null = null;
  let heldLeave: ReturnType<typeof setTimeout> | null = null;
  const atOuterEdge = (p: { x: number } | null) =>
    !!p && (cfg.side === "right" ? p.x >= innerWidth - 12 : p.x <= 12);
  // After GTK reports the pointer left, WebKit can still deliver stale mousemove events at the old
  // position (seen as the tab card refusing to hide). Once native crossings are known to work,
  // DOM pointer events only count between a native enter and leave.
  let nativeCrossings = false;
  let nativeInside = false;
  function onNativeLeave() {
    nativeInside = false;
    if (heldLeave) clearTimeout(heldLeave);
    heldLeave = null;
    const inGrace = mode === "expanded" && performance.now() - openedAt < GRACE_MS;
    if (inGrace && atOuterEdge(lastMove)) {
      log(`dropped leave just after opening, pointer at edge ${lastMove?.x.toFixed(0)}`);
      return;
    }
    if (inGrace) {
      heldLeave = setTimeout(() => {
        heldLeave = null;
        setHover(null);
        edge.pointerLeave();
      }, GRACE_MS);
      return;
    }
    setHover(null);
    edge.pointerLeave();
  }
  function onNativeEnter() {
    nativeCrossings = nativeInside = true;
    if (heldLeave) clearTimeout(heldLeave);
    heldLeave = null;
  }

  // The tab preview hides only after the pointer has been off every tab for a moment, so a mouse
  // wobbling across the strip's inner edge does not make the card blink.
  const HOVER_HIDE_MS = 220;
  let hoverHide: ReturnType<typeof setTimeout> | null = null;
  function setHover(id: string | null) {
    if (id) {
      if (hoverHide) clearTimeout(hoverHide);
      hoverHide = null;
      hoverTab = id;
    } else if (hoverTab && !hoverHide) {
      hoverHide = setTimeout(() => {
        hoverHide = null;
        hoverTab = null;
      }, HOVER_HIDE_MS);
    }
  }

  const setWindow = (expanded: boolean, focus: boolean) =>
    (windowQueue = windowQueue.then(() => invoke("set_expanded", { expanded, focus })).catch((e) => log(`${e}`)));

  async function changeMode(m: Mode, reason: Reason) {
    const mine = ++seq;
    log(`mode ${m} (${reason})`);
    mode = m;
    if (m === "expanded") {
      openedAt = performance.now();
      // Opening from a tab: switch to that note BEFORE the slide starts, so the panel never
      // shows the list for a frame and then jumps to the editor.
      const target = pendingOpen;
      pendingOpen = null;
      if (pendingSaved) {
        pendingSaved = false;
        settingsOpen = false;
        if (editing) void flushSave();
        editing = null;
        tab = "saved";
        folder = null;
        query = "";
        void refresh();
      }
      if (target) {
        settingsOpen = false;
        if (editing && draft !== editing.body) void flushSave();
        editing = target;
        draft = target.body;
      }
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
      // The hovered tab's card stays until the panel has slid over it, then goes.
      void sleep(OPEN_MS).then(() => {
        if (mine === seq) hoverTab = null;
      });
      if (target) {
        await tick();
        editorEl?.focus();
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
      hoverTab = null;
      (document.activeElement as HTMLElement | null)?.blur(); // never type into a hidden panel
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
    notes = query.trim() && tab === "notes" ? await invoke<Note[]>("notes_search", { query }) : all;
    allSaves = await invoke<Note[]>("saves_list");
    found = query.trim() && tab === "saved" ? await invoke<Note[]>("saves_search", { query }) : allSaves;
    [folders, defaultFolders] = await invoke<[[string, number][], string[]]>("save_folders");
    const midnight = new Date();
    midnight.setHours(0, 0, 0, 0);
    today = await invoke<Note[]>("saves_today", { todayStart: midnight.getTime() });
    if (folder && !folders.some(([f]) => f === folder)) folder = null;
    if (folder === "" && !allSaves.some((n) => !n.link?.folder)) folder = null;
    if (!flat.some((n) => n.id === selectedId)) selectedId = flat[0]?.id ?? null;
  }

  /** Put an updated note into both lists at once, so tabs and cards change without a reload. */
  function patch(n: Note) {
    all = all.map((x) => (x.id === n.id ? n : x));
    notes = notes.map((x) => (x.id === n.id ? n : x));
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
      patch(editing);
    }
  }

  async function addQuick() {
    const body = quick.trim();
    if (!body) return;
    if (isOnlyLink(body)) {
      await invoke("save_create", { text: body });
      tab = "saved";
    } else {
      await invoke("note_create", { body });
    }
    quick = "";
    await refresh();
  }

  function showTab(t: typeof tab) {
    tab = t;
    void refresh();
  }

  async function openSave(n: Note) {
    try {
      const updated = await invoke<Note>("save_open", { id: n.id });
      found = found.map((x) => (x.id === n.id ? updated : x));
    } catch (e) {
      log(`open save: ${e}`);
    }
  }

  /** Moves a save; "" is Unsorted, "\u0001new" asks for a name. False if the user cancelled. */
  async function moveSave(n: Note, to: string): Promise<boolean> {
    const name = to === "\u0001new" ? prompt("New folder name")?.trim().slice(0, 40) : to;
    if (!name && to !== "") return false;
    await invoke("save_set_folder", { id: n.id, folder: to === "" ? null : name });
    await refresh();
    return true;
  }

  async function markDone(n: Note) {
    await invoke("save_set_done", { id: n.id, done: !n.link?.done });
    await refresh();
  }

  async function deleteSave(n: Note) {
    await invoke("note_delete", { id: n.id });
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
    return el.dataset.edge === "tab" || el.dataset.edge === "peek" ? { kind: "tab", id: el.dataset.id! } : { kind: "bar" };
  }

  /** While collapsed only the main bar (and +N) opens on hover; a tab shows its title. */
  function domPointerAt(x: number, y: number, entering: boolean) {
    if (nativeCrossings && !nativeInside) return;
    pointerAt(x, y, entering);
  }

  function pointerAt(x: number, y: number, entering: boolean) {
    if (mode === "expanded") {
      if (entering) edge.pointerEnter();
      else edge.pointerMove();
      return;
    }
    const t = targetAt(x, y);
    setHover(t?.kind === "tab" ? t.id : null);
    if (t?.kind === "bar") {
      if (entering) edge.pointerEnter();
      else edge.pointerMove();
    }
  }

  function openTab(n: Note) {
    pendingOpen = all.find((x) => x.id === n.id) ?? n;
    edge.click();
  }

  const hovered = $derived(edgeItems.tabs.find((n) => n.id === hoverTab) ?? null);
  // While the card shows (and the panel is collapsed), Rust adds its box to the clickable area.
  let peekEl: HTMLElement | undefined = $state();
  $effect(() => {
    const el = peekEl;
    if (!hovered || !el || mode === "expanded") {
      void invoke("set_peek", { rect: null }).catch(() => {});
      return;
    }
    const r = el.getBoundingClientRect();
    void invoke("set_peek", { rect: { x: r.left, y: r.top, w: r.width, h: r.height } }).catch(() => {});
  });

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
  $effect(() => {
    if (shown) arrived = false;
  });

  async function syncStep(action: () => Promise<void>) {
    syncBusy = true;
    syncMsg = "";
    try {
      await action();
    } catch (e) {
      syncMsg = String(e);
    }
    syncBusy = false;
  }
  // Opens the browser at Google; resolves when the user is back and signed in.
  const signInGoogle = () => syncStep(async () => void (sync = await invoke<SyncStatus>("sync_sign_in_google")));
  const signOutSync = () => syncStep(async () => void (sync = await invoke<SyncStatus>("sync_sign_out")));
  const deleteAccount = () => {
    if (!confirm(`Delete the Edge account ${sync.email}? Everything synced to it is deleted from the server and every device is signed out. Notes on this laptop stay.`)) return;
    void syncStep(async () => void (sync = await invoke<SyncStatus>("sync_delete_account")));
  };

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

  // Hover opens the panel without keyboard focus (so typing stays in your app). A click inside asks
  // for focus, then puts the caret where you clicked; GNOME does not focus a dock on click by itself.
  function claimFocus(e: PointerEvent) {
    if (document.hasFocus()) return;
    const target = e.target as HTMLElement;
    void invoke("focus_panel").then(() => {
      const field = target.closest<HTMLElement>("input, textarea");
      if (field && document.activeElement !== field) field.focus();
    });
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
    if (editing || settingsOpen || tab === "saved" || document.activeElement === quickEl) return;
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
    log("page ready");
    invoke<{ focusOnHover: boolean; fixedWindow: boolean; os: typeof os }>("platform").then((p) => {
      focusOnHover = p.focusOnHover;
      fixedWindow = p.fixedWindow;
      os = p.os;
    });
    void invoke<Settings>("get_settings").then((c) => (cfg = c));
    void listen("edge-hotkey", () => edge.hotkey());
    void invoke<SyncStatus>("sync_status").then((st) => (sync = st));
    void listen("saves-updated", () => void refresh());
    void listen<SyncStatus>("sync-status", (e) => {
      sync = e.payload;
      if (e.payload.received > 0) {
        void refresh();
        if (!shown) arrived = true;
      }
    });
    void listen<HotkeyStatus>("edge-hotkey-status", (e) => (hotkey = e.payload));
    window.addEventListener("error", (e) => log(`error: ${e.message}`));
    window.addEventListener("unhandledrejection", (e) => log(`rejection: ${e.reason}`));
    // WebKitGTK under XWayland drops DOM mouseleave; GTK's own crossings (with x/y) are reliable.
    void listen<{ kind: "enter" | "leave" | "move"; x: number; y: number }>("edge-pointer", (e) => {
      if (e.payload.kind === "leave") {
        onNativeLeave();
      } else {
        if (e.payload.kind === "enter") onNativeEnter();
        else lastMove = { x: e.payload.x, y: e.payload.y };
        pointerAt(e.payload.x, e.payload.y, e.payload.kind === "enter");
      }
    });
    document.addEventListener("mouseout", (e) => {
      if (!e.relatedTarget) {
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
        placeholder={tab === "saved" ? `Search ${allSaves.length} saves` : `Search ${all.length} notes`}
        aria-label={tab === "saved" ? "Search saves" : "Search notes"}
      />
    </div>
    <button class="ib" onclick={openSettings} aria-label="Settings" title="Settings"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg></button>
    <button class="ib" onclick={() => edge.escape()} aria-label="Close panel" title="Close"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
  </header>
  <nav class="tabs" aria-label="Notes or saved posts">
    <button class:on={tab === "notes"} onclick={() => showTab("notes")}>Notes</button>
    <button class:on={tab === "saved"} onclick={() => showTab("saved")}>Saved{#if allSaves.length}<span>{allSaves.length}</span>{/if}</button>
  </nav>
{/snippet}

{#snippet saveRow(n: Note, forToday: boolean)}
      {@const l = n.link!}
      {@const p = PLATFORMS[l.platform]}
      <div class="save" class:done={l.done} data-id={n.id}>
        <button class="open" onclick={() => openSave(n)} title="Open in {platformName(l)}">
          <span class="lead">
            {#if l.thumbnail}
              <img src={l.thumbnail} alt="" loading="lazy" referrerpolicy="no-referrer" onerror={(e) => ((e.currentTarget as HTMLImageElement).style.display = "none")} />
            {/if}
            <span class="mark" class:small={!!l.thumbnail} class:mono={p?.color === "#101010"} style:--c={p?.color ?? "var(--accent)"}>{p?.mark ?? "↗"}</span>
          </span>
          <span class="body">
            <span class="st">{saveTitle(n)}</span>
            <span class="meta">{platformName(l)}{l.author ? ` · ${l.author}` : ""} · {ago(n.created_at)}{l.done ? " · done" : l.opened_at ? " · opened" : ""}{folder === null && l.folder ? ` · ${l.folder}` : ""}</span>
          </span>
        </button>
        <select class="move" aria-label="Folder" title="Move to a folder" value={l.folder ?? ""}
          onchange={(e) => {
            const el = e.currentTarget as HTMLSelectElement;
            void moveSave(n, el.value).then((moved) => { if (!moved) el.value = l.folder ?? ""; });
          }}>
          <option value="">Unsorted</option>
          {#each [...new Set([...defaultFolders, ...folders.map(([f]) => f), ...(l.folder ? [l.folder] : [])])] as f (f)}
            <option value={f}>{f}</option>
          {/each}
          <option value={"\u0001new"}>New folder…</option>
        </select>
        <button class="ib del" class:keep={forToday} onclick={() => markDone(n)} aria-label={l.done ? "Not done" : "Mark done"} title={l.done ? "Not done" : "Done"}><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg></button>
        <button class="ib del" onclick={() => deleteSave(n)} aria-label="Delete save" title="Delete"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
      </div>
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
      placeholder={tab === "saved" ? "Paste a link to save it" : variant === "slim" ? "+ New note" : variant === "tile" ? "+ New note" : "Write a thought…"}
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
  onmouseenter={(e) => {
    log(`mouseenter ${e.clientX},${e.clientY}`); // debug builds only: the Windows smoke test reads it
    domPointerAt(e.clientX, e.clientY, true);
  }}
  onmouseleave={() => {
    hoverTab = null;
    edge.pointerLeave();
  }}
  onmousemove={(e) => domPointerAt(e.clientX, e.clientY, false)}
>
  <div
    class="strip"
    ondragstart={(e) => e.preventDefault()}
    class:hidden={shown || !cfg.showBar}
    style:height="min({stripHeight}px, 100vh)"
    style:top="calc((100vh - min({stripHeight}px, 100vh)) * {cfg.barPosition})"
  >
    {#each edgeItems.tabs as n (n.id)}
      <button class="tab" class:hover={hoverTab === n.id} data-edge="tab" data-id={n.id} style:--c={n.color ?? "var(--bar)"}
        onclick={() => openTab(n)} aria-label="Open {n.title}"></button>
    {/each}
    {#if edgeItems.more}
      <button class="tab more" data-edge="bar" onclick={() => edge.click()} aria-label="{edgeItems.more} more pinned notes">
        <span>+{edgeItems.more}</span>
      </button>
    {/if}
    {#if today.length}
      <button class="tab saved" data-edge="tab" onclick={() => { pendingSaved = true; edge.click(); }}
        aria-label="{today.length} saves to come back to" title="{today.length} saves to come back to today">
        <span>{today.length}</span>
      </button>
    {/if}
    <button class="bar" data-edge="bar" onclick={() => edge.click()} aria-label="Open notes ({all.length}){arrived ? ', new from another device' : ''}">
      {#if arrived}<i class="arrived"></i>{/if}
      {#if all.length}<span>{all.length}</span>{/if}
    </button>
  </div>

  {#if hovered}
    {@const items = checklistItems(hovered)}
    {@const tabIndex = edgeItems.tabs.indexOf(hovered)}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="peek"
      bind:this={peekEl}
      onclick={() => hovered && openTab(hovered)}
      data-edge="peek"
      data-id={hovered.id}
      style:--c={hovered.color ?? "var(--bar)"}
      style:top="clamp(8px, calc((100vh - min({stripHeight}px, 100vh)) * {cfg.barPosition} + {tabIndex * 42}px - 40px), calc(100vh - 340px))"
    >
      <div class="peek-t">{hovered.title || "Untitled"}</div>
      <div class="peek-b">
        {#each hovered.body.split("\n").slice(1) as line}
          {@const item = /^\s*- \[([ xX])\] (.*)$/.exec(line)}
          {#if item}<div class="ci" class:done={item[1] !== " "}>{item[1] !== " " ? "☑" : "☐"} {item[2]}</div>
          {:else if line.trim()}<div>{line}</div>
          {:else}<div class="gap"></div>{/if}
        {/each}
      </div>
      <div class="peek-f">{items.length ? `${items.filter((i) => i.done).length}/${items.length} done · ` : ""}Click to edit</div>
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="panel" class:shown style:--open-ms="{OPEN_MS}ms" style:--close-ms="{CLOSE_MS}ms" onpointerdown={claimFocus}>

    {#if settingsOpen}
      <header class="top">
        <button class="ib" onclick={() => (settingsOpen = false)} aria-label="Back to notes"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 18-6-6 6-6"/></svg></button>
        <h1 class="grow">Settings</h1>
        <button class="ib" onclick={() => edge.escape()} aria-label="Close panel"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
      </header>
      <div class="settings">
        <div class="group">Sync</div>
        {#if sync.email}
          <section class="setting">
            <div class="grow">
              <div class="ellipsis">{sync.email}</div>
              <div class="sub">{sync.lastSyncedAt ? `Synced ${ago(sync.lastSyncedAt) === "now" ? "just now" : ago(sync.lastSyncedAt) + " ago"}` : "Not synced yet"}</div>
            </div>
            <button class="pill" disabled={syncBusy} onclick={() => invoke("sync_now")}>Sync now</button>
          </section>
          {#if sync.error}<p class="error pad">{sync.error}</p>{/if}
          <section class="setting">
            <div class="grow sub">Notes stay on this laptop if you sign out.</div>
            <button class="pill" disabled={syncBusy} onclick={signOutSync}>Sign out</button>
          </section>
          <section class="setting">
            <div class="grow sub">Delete the account and everything synced to it.</div>
            <button class="pill danger" disabled={syncBusy} onclick={deleteAccount}>Delete account</button>
          </section>
        {:else}
          <section class="setting col">
            <div class="sub">Get your phone's notes here. Free for 14 days, no card needed.</div>
            <button class="google" disabled={syncBusy} onclick={signInGoogle}>
              <svg viewBox="0 0 48 48" width="16" height="16" aria-hidden="true"><path fill="#FFC107" d="M43.6 20.5H42V20H24v8h11.3C33.7 32.7 29.2 36 24 36c-6.6 0-12-5.4-12-12s5.4-12 12-12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 12.9 4 4 12.9 4 24s8.9 20 20 20 20-8.9 20-20c0-1.3-.1-2.4-.4-3.5z"/><path fill="#FF3D00" d="m6.3 14.7 6.6 4.8C14.7 15.1 19 12 24 12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 16.3 4 9.7 8.3 6.3 14.7z"/><path fill="#4CAF50" d="M24 44c5.2 0 9.9-2 13.4-5.2l-6.2-5.2C29.2 35.1 26.7 36 24 36c-5.2 0-9.6-3.3-11.3-8l-6.5 5C9.5 39.6 16.2 44 24 44z"/><path fill="#1976D2" d="M43.6 20.5H42V20H24v8h11.3c-.8 2.2-2.2 4.2-4.1 5.6l6.2 5.2C37 39.2 44 34 44 24c0-1.3-.1-2.4-.4-3.5z"/></svg>
              {syncBusy ? "Finish signing in in your browser…" : "Continue with Google"}
            </button>
          </section>
          {#if sync.error}<p class="error pad">{sync.error}</p>{/if}
        {/if}
        {#if syncMsg}<p class="error pad" role="status">{syncMsg}</p>{/if}

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
    {:else if tab === "saved"}
      {@render header()}
      {#if allSaves.length}
        <div class="folders" aria-label="Folders">
          <button class:on={folder === null} onclick={() => (folder = null)}>All <span>{allSaves.length}</span></button>
          {#each folders as [f, c] (f)}
            <button class:on={folder === f} onclick={() => (folder = f)}>{f} <span>{c}</span></button>
          {/each}
          {#if unsorted}<button class:on={folder === ""} onclick={() => (folder = "")}>Unsorted <span>{unsorted}</span></button>{/if}
        </div>
      {/if}
      <div class="saves">
        {#if today.length && folder === null && !query.trim()}
          <div class="sec">For today</div>
          {#each today as n (n.id)}
            {@render saveRow(n, true)}
          {/each}
          <div class="sec">All saves</div>
        {/if}
        {#each saves as n (n.id)}
          {@render saveRow(n, false)}
        {:else}
          <div class="empty">{query ? "No matches" : "Share a post to Edge on your phone, or paste a link below. Saves land here."}</div>
        {/each}
      </div>
      {@render capture("slim")}
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
  .pill.danger { color: #e5484d; }
  .tabs { display: flex; gap: 4px; padding: 0 2px 8px; }
  .tabs button { font: inherit; font-size: 12.5px; padding: 4px 10px; border-radius: 999px; color: var(--muted); background: none; }
  .tabs button:hover { background: var(--surface-2); }
  .tabs button.on { background: var(--surface-2); color: var(--text); font-weight: 600; }
  .tabs span { margin-left: 6px; color: var(--faint); font-weight: 500; }
  .saves { flex: 1; min-height: 0; overflow: auto; margin: 0 -4px; }
  .save { display: flex; align-items: center; border-radius: 10px; }
  .save:hover { background: var(--surface-2); }
  .save .open { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 8px 6px 8px 8px; background: none; text-align: left; font: inherit; color: inherit; cursor: pointer; }
  .mark { width: 28px; height: 28px; border-radius: 50%; flex: none; display: grid; place-items: center; background: var(--c); color: #fff; font-size: 11.5px; font-weight: 700; }
  .mark.mono { background: var(--text); color: var(--bg); } /* Threads and X: black on light, light on dark */
  .body { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .st { font-weight: 500; overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; line-height: 1.3; }
  .meta { color: var(--faint); font-size: 11.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .save .del { opacity: 0; margin-right: 4px; }
  .save .del.keep { opacity: 0.7; }
  .save.done { opacity: 0.55; }
  .saves .sec { font-size: 11px; font-weight: 600; color: var(--faint); padding: 10px 8px 4px; letter-spacing: .05em; text-transform: uppercase; }
  .save:hover .del, .save .del:focus-visible { opacity: 1; }
  .folders { display: flex; gap: 6px; overflow-x: auto; padding: 0 2px 8px; scrollbar-width: none; }
  .folders button { font: inherit; font-size: 12px; white-space: nowrap; padding: 3px 9px; border-radius: 999px; border: 1px solid var(--line); background: none; color: var(--muted); }
  .folders button.on { border-color: var(--accent); color: var(--text); font-weight: 600; }
  .folders span { color: var(--faint); font-weight: 500; margin-left: 3px; }
  .lead { position: relative; width: 40px; height: 40px; flex: none; display: grid; place-items: center; }
  .lead img { width: 40px; height: 40px; border-radius: 8px; object-fit: cover; }
  .lead .mark.small { position: absolute; right: -4px; bottom: -4px; width: 18px; height: 18px; font-size: 8.5px; box-shadow: 0 0 0 2px var(--bg); }
  .move { opacity: 0; max-width: 92px; font: inherit; font-size: 11.5px; color: var(--muted); background: var(--surface); border: 1px solid var(--line); border-radius: 6px; padding: 2px 4px; }
  .save:hover .move, .move:focus { opacity: 1; }
  .empty { color: var(--faint); padding: 24px 12px; text-align: center; line-height: 1.4; }
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
    /* No text selection or drag gestures on the edge: a drag starts a pointer grab that left
       GNOME/XWayland with the panel half drawn. */
    user-select: none;
    -webkit-user-drag: none;
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
  .tab.saved {
    background: var(--accent);
    display: grid;
    place-items: center;
  }
  .tab.saved span,
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
  /* Preview of the hovered tab: the whole note, beside the edge, clickable to edit. */
  .peek {
    position: fixed;
    right: 8px;
    width: 300px;
    max-height: 320px;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--text);
    border-radius: 12px 0 12px 12px;
    border-top: 4px solid var(--c);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.25);
    padding: 10px 14px 8px;
    box-sizing: border-box;
    cursor: pointer;
    animation: peek 0.12s ease-out;
  }
  @keyframes peek {
    from {
      opacity: 0;
      transform: translateX(8px);
    }
  }
  .peek-t {
    font-weight: 600;
    margin-bottom: 4px;
  }
  .peek-b {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    color: var(--muted);
    font-size: 13px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    -webkit-mask-image: linear-gradient(to bottom, #000 85%, transparent);
    mask-image: linear-gradient(to bottom, #000 85%, transparent);
  }
  .peek-b .gap {
    height: 6px;
  }
  .peek-b .ci.done {
    color: var(--faint);
    text-decoration: line-through;
  }
  .peek-f {
    margin-top: 6px;
    font-size: 11px;
    color: var(--faint);
  }
  .left .strip {
    right: auto;
    left: 0;
  }
  .left .tab,
  .left .bar {
    border-radius: 0 5px 5px 0;
  }
  .left .peek {
    right: auto;
    left: 8px;
    border-radius: 0 12px 12px 12px;
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
  .arrived {
    position: absolute;
    top: 6px;
    left: 1px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #22c55e;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.25);
  }
  .pill {
    flex: none;
    padding: 5px 11px;
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text);
    font-size: 12.5px;
  }
  .pill:disabled {
    opacity: 0.5;
  }
  .google {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
    padding: 8px 12px;
    border-radius: 9px;
    border: 1px solid var(--line);
    background: var(--surface);
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
  }
  .google:disabled {
    opacity: 0.6;
  }
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
