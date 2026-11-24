<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { createEdge, type Mode, type Reason } from "$lib/hover";
  import { captureHotkey, formatHotkey } from "$lib/accel";
  import { ago, checklistItems, edgeTabs, nextColor, PALETTE, sections as toSections, onOpenNoteChanged, type Note, type Attachment, liveAttachments, isOnlyLink, platformName, saveTitle, PLATFORMS } from "$lib/notes";
  import Cards from "$lib/views/Cards.svelte";
  import Compact from "$lib/views/Compact.svelte";
  import Sticky from "$lib/views/Sticky.svelte";
  import Onboarding from "$lib/Onboarding.svelte";
  import RichEditor from "$lib/RichEditor.svelte";
  import Attachments from "$lib/Attachments.svelte";
  import Agents from "$lib/Agents.svelte";
  import Licenses from "$lib/Licenses.svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { plain } from "$lib/format";
  import "@fontsource-variable/figtree";
  import "@fontsource/instrument-serif/400.css";
  import "@fontsource/instrument-serif/400-italic.css";

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
    onboarded: boolean;
    mcpAccess: "off" | "read" | "write" | "full";
  };
  type HotkeyStatus = { accelerator: string; bound: string | null; portal: boolean; error: string | null };

  // ---- state
  let mode: Mode = $state(new URLSearchParams(location.search).get("w") === "home" ? "expanded" : "collapsed");
  let all: Note[] = $state([]); // every visible note, for the edge tabs
  let notes: Note[] = $state([]); // what the list shows (search results while searching)
  let query = $state("");
  let tab: "notes" | "saved" = $state("notes");
  /** The app window's Agents page (MCP access for AI agents). */
  let agentsOpen = $state(false);
  let licensesOpen = $state(false);
  let appVersion = $state("");
  void getVersion().then((v) => (appVersion = v));
  let allSaves: Note[] = $state([]);
  let found: Note[] = $state([]); // saves matching the search
  let folders: [string, number][] = $state([]);
  let defaultFolders: string[] = $state([]);
  let today: Note[] = $state([]); // saves to bring back today, also the bookmark tab on the edge
  /** The right-click menu: which note or save, and where. */
  let menu: { n: Note; x: number; y: number } | null = $state(null);
  // The app's own confirm / prompt: window.confirm and prompt show nothing in the Linux webview
  // (they return false / null at once), which made Delete do nothing.
  let asking: { text: string; ok: string; input: string | null; done: (v: string | null) => void } | null = $state(null);
  function ask(text: string, ok: string, input: string | null = null): Promise<string | null> {
    log(`ask: ${text}`);
    return new Promise((done) => (asking = { text, ok, input, done }));
  }
  function answer(v: string | null) {
    log(`answer: ${JSON.stringify(v)} (asking ${asking ? "set" : "null"})`);
    asking?.done(v);
    asking = null;
  }
  /** A downloaded update waiting for a restart, or one this install can't apply itself. */
  let update: { version: string; ready: boolean; url?: string } | null = $state(null);
  let pendingSaved = false; // the edge's bookmark tab was clicked: open on Saved
  let folder: string | null = $state(null); // the chosen folder chip; null: all, "" : unsorted
  const saves = $derived(folder === null ? found : found.filter((n) => (n.link?.folder ?? "") === folder));
  const unsorted = $derived(allSaves.filter((n) => !n.link?.folder).length);
  let selectedId: string | null = $state(null);
  let editing: Note | null = $state(null);
  let draft = $state("");
  let quick = $state("");
  let searchEl: HTMLInputElement | undefined = $state();
  let editorEl: HTMLDivElement | undefined = $state();
  let rich: RichEditor | undefined = $state();
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
    onboarded: true, // until loaded: never flash the tour at someone who finished it
    mcpAccess: "off",
  });
  let cfgLoaded = $state(false);
  /** The app window (home.rs) shows the same panel as a normal window: no edge, no hover. */
  const standalone = new URLSearchParams(location.search).get("w") === "home";
  const openHome = () => {
    edge.escape();
    void invoke("open_home");
  };
  let autostart: boolean | null = $state(null);
  let settingsMsg = $state("");

  // ---- sync (runs in Rust; the page shows its status and does sign-in)
  type SyncStatus = {
    email: string | null; lastSyncedAt: number | null; error: string | null; received: number; beta: boolean;
    trialEndsAt: number | null; paidUntil: number | null; billing: boolean;
  };
  let sync: SyncStatus = $state({ email: null, lastSyncedAt: null, error: null, received: 0, beta: false, trialEndsAt: null, paidUntil: null, billing: false });
  /** Saved posts: for everyone since v1. */
  const savesOn = true;
  /** Settings → Sync: the plan in one line, e.g. "Free trial · 23 days left". */
  const plan = $derived.by(() => {
    const now = Date.now();
    const days = (t: number) => Math.max(1, Math.ceil((t - now) / 86_400_000));
    if (sync.paidUntil && sync.paidUntil > now) return { paid: true, text: `Subscribed · renews or ends in ${days(sync.paidUntil)} days` };
    if (sync.trialEndsAt && sync.trialEndsAt > now) return { paid: false, text: `Free trial · ${days(sync.trialEndsAt)} days left` };
    return { paid: false, text: "Trial ended · sync and AI agents are paused, notes stay here" };
  });
  async function billing(kind: "monthly" | "yearly" | "portal") {
    syncMsg = "";
    try {
      await (kind === "portal" ? invoke("billing_portal") : invoke("billing_checkout", { plan: kind }));
      syncMsg = kind === "portal" ? "" : "Finish in your browser; sync turns on within a minute.";
    } catch (e) {
      syncMsg = `${e}`;
    }
  }
  /** The server refuses this version (426): the updater runs; say so. */
  const tooOld = $derived(!!sync.error && sync.error.includes("too old"));
  $effect(() => {
    if (!savesOn && tab === "saved") tab = "notes";
  });
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
  const tabCount = $derived(edgeItems.tabs.length + (edgeItems.more ? 1 : 0) + (today.length && savesOn ? 1 : 0));
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
  let shown = $state(new URLSearchParams(location.search).get("w") === "home");
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
  // The app's text menu (right-click in the editor, search or quick box).
  let textMenu: { x: number; y: number; field: HTMLElement; href?: string } | null = $state(null);
  const inEditor = (f: HTMLElement) => f.isContentEditable;
  async function clipWrite(text: string) {
    try {
      await invoke("clip_write", { text });
    } catch {
      await navigator.clipboard.writeText(text);
    }
  }
  async function clipRead(): Promise<string> {
    try {
      return await invoke<string>("clip_read");
    } catch {
      return navigator.clipboard.readText();
    }
  }
  function fieldSelection(f: HTMLElement): string {
    if (inEditor(f)) return rich?.selection() ?? "";
    const i = f as HTMLInputElement;
    return i.value.slice(i.selectionStart ?? 0, i.selectionEnd ?? 0);
  }
  function fieldInsert(f: HTMLElement, text: string) {
    if (inEditor(f)) return rich?.insert(text);
    const i = f as HTMLInputElement;
    i.focus();
    i.setRangeText(text, i.selectionStart ?? i.value.length, i.selectionEnd ?? i.value.length, "end");
    i.dispatchEvent(new Event("input", { bubbles: true })); // so bind:value sees it
  }
  async function textAction(kind: "cut" | "copy" | "paste" | "all") {
    const f = textMenu?.field;
    textMenu = null;
    if (!f) return;
    try {
      if (kind === "all") {
        if (inEditor(f)) rich?.selectAll();
        else (f.focus(), (f as HTMLInputElement).select());
      } else if (kind === "paste") {
        fieldInsert(f, (await clipRead()).replace(/\r\n?/g, "\n"));
      } else {
        const text = fieldSelection(f);
        if (!text) return;
        await clipWrite(text);
        if (kind === "cut") fieldInsert(f, "");
      }
    } catch (e) {
      log(`text menu ${kind}: ${e}`);
    }
  }
  /** Leaving the window closes the panel. */
  // Hold: the panel stays open while you work elsewhere (pointer away, another app focused).
  // Only Escape, the × button, the hotkey or turning hold off closes it. Not saved: every new
  // session starts with the panel behaving normally.
  let held = $state(false);
  function leave() {
    if (held) return;
    edge.pointerLeave();
  }

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
        leave();
      }, GRACE_MS);
      return;
    }
    setHover(null);
    leave();
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
      if (reason === "hotkey") pendingSaved = false; // the shortcut is for writing: Notes
      if (pendingSaved) {
        pendingSaved = false;
        settingsOpen = false;
        if (editing) void flushSave();
        editing = null;
        tab = "saved";
        folder = null;
        query = "";
        void refresh();
      } else if (!target && tab === "saved") {
        // The bar (or the shortcut) always opens on Notes; only the bookmark tab opens on Saved.
        tab = "notes";
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
      held = false;
      // A dialog or menu left open would stay drawn over the desktop after the panel closes
      // (the window keeps a dialog-sized hit area), and no click outside reaches it: cancel them.
      if (asking) answer(null);
      menu = null;
      textMenu = null;
      attachMenu = false;
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
    unseen = new Set(await invoke<string[]>("pins_unseen"));
    notes = query.trim() && tab === "notes" ? await invoke<Note[]>("notes_search", { query }) : all;
    allSaves = await invoke<Note[]>("saves_list");
    followOpenNote();
    found = query.trim() && tab === "saved" ? await invoke<Note[]>("saves_search", { query }) : allSaves;
    [folders, defaultFolders] = await invoke<[[string, number][], string[]]>("save_folders");
    const midnight = new Date();
    midnight.setHours(0, 0, 0, 0);
    today = await invoke<Note[]>("saves_today", { todayStart: midnight.getTime() });
    if (folder && !folders.some(([f]) => f === folder)) folder = null;
    if (folder === "" && !allSaves.some((n) => !n.link?.folder)) folder = null;
    if (!flat.some((n) => n.id === selectedId)) selectedId = flat[0]?.id ?? null;
  }

  /** Pinned on another device and not looked at here yet: their tabs and cards call for attention. */
  let unseen: Set<string> = $state(new Set());
  function seen(id: string) {
    if (!unseen.has(id)) return;
    unseen = new Set([...unseen].filter((x) => x !== id));
    void invoke("pin_seen", { id }).catch((e) => log(`pin_seen: ${e}`));
  }

  /** Put an updated note into both lists at once, so tabs and cards change without a reload. */
  function patch(n: Note) {
    all = all.map((x) => (x.id === n.id ? n : x));
    notes = notes.map((x) => (x.id === n.id ? n : x));
  }

  async function open(n: Note) {
    seen(n.id);
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

  /** The open note was changed elsewhere (sync, an agent, the other window): show the new text
   *  unless you are typing, in which case your text is saved as the newer edit. */
  function followOpenNote() {
    const open = editing;
    if (!open || saveTimer) return; // typing right now: the save below makes the newer edit
    const latest = [...all, ...allSaves].find((n) => n.id === open.id);
    switch (onOpenNoteChanged(open, latest, draft)) {
      case "load":
        editing = latest!;
        draft = latest!.body;
        break;
      case "closed":
        editing = null;
        break;
    }
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
    // Only the Saved tab turns a pasted link into a save; on Notes, a link is just a note.
    if (savesOn && tab === "saved" && isOnlyLink(body)) {
      await invoke("save_create", { text: body });
    } else {
      await invoke("note_create", { body });
    }
    quick = "";
    await refresh();
  }

  /** The quick box's text (or nothing) as a new note, opened in the full editor. */
  async function openFull() {
    const n = await invoke<Note>("note_create", { body: quick });
    quick = "";
    await refresh();
    await open(n);
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
    const name = to === "\u0001new" ? (await ask("New folder name", "Create", ""))?.trim().slice(0, 40) : to;
    if (!name && to !== "") return false;
    await invoke("save_set_folder", { id: n.id, folder: to === "" ? null : name });
    await refresh();
    return true;
  }

  async function deleteNote(n: Note) {
    if ((await ask(`Delete "${n.title || "Untitled"}"?`, "Delete")) === null) return;
    log(`deleting ${n.id}`);
    await invoke("note_delete", { id: n.id });
    if (editing?.id === n.id) editing = null;
    await refresh();
  }

  // ---- attachments (images and recordings, up to 50 MB each)
  const MAX_BYTES = 50 * 1024 * 1024;
  let fileInput: HTMLInputElement | undefined = $state();
  let galleryInput: HTMLInputElement | undefined = $state();
  let attachMenu = $state(false);
  let attaching = $state("");
  /** While the Bar position slider moves: the value it shows, before it is saved. */
  let barPreview: number | null = $state(null);

  /** Width and height of an image, or the length of a recording, read in the page. */
  async function measure(f: File): Promise<{ width?: number; height?: number; duration?: number }> {
    const url = URL.createObjectURL(f);
    try {
      if (f.type.startsWith("image/")) {
        const img = new Image();
        img.src = url;
        await img.decode();
        return { width: img.naturalWidth, height: img.naturalHeight };
      }
      const a = new Audio();
      a.preload = "metadata";
      a.src = url;
      await new Promise((ok) => ((a.onloadedmetadata = ok), (a.onerror = ok), setTimeout(ok, 3000)));
      return Number.isFinite(a.duration) ? { duration: Math.round(a.duration * 1000) } : {};
    } catch {
      return {};
    } finally {
      URL.revokeObjectURL(url);
    }
  }

  /** Attaches files to `target` (the open note by default; a recording keeps the note it began in). */
  async function attach(files: File[], target: Note | null = editing) {
    if (!target) return;
    if (editing?.id === target.id) await flushSave();
    for (const f of files) {
      if (f.size > MAX_BYTES) {
        attaching = `${f.name} is ${(f.size / 1e6).toFixed(0)} MB. Attachments can be up to 50 MB.`;
        continue;
      }
      attaching = `Adding ${f.name || "image"}…`;
      const m = await measure(f);
      const headers: Record<string, string> = { "x-note": target.id, "x-mime": f.type || "application/octet-stream", "x-name": encodeURIComponent(f.name) };
      if (m.width) headers["x-width"] = String(m.width);
      if (m.height) headers["x-height"] = String(m.height);
      if (m.duration) headers["x-duration"] = String(m.duration);
      try {
        const n = await invoke<Note>("note_attach", new Uint8Array(await f.arrayBuffer()), { headers });
        if (editing?.id === n.id) editing = n;
        patch(n);
        attaching = "";
      } catch (e) {
        attaching = `Could not add ${f.name}: ${e}`;
      }
    }
  }

  // A note opened and left without text or attachments (Full editor, then away) is removed,
  // whichever way the editor was left: Back, another note, the rail, the panel closing.
  $effect(() => {
    const keep = editing?.id ?? null;
    void invoke<number>("notes_clean", { keep })
      .then((n) => { if (n > 0) void refresh(); })
      .catch((e) => log(`notes_clean: ${e}`));
  });

  async function movePin(id: string, before: string | null) {
    const n = await invoke<Note>("note_move_pin", { id, before });
    patch(n);
    await refresh();
  }

  async function detach(a: Attachment) {
    if (!editing) return;
    editing = await invoke<Note>("note_detach", { id: editing.id, attachment: a.id });
    patch(editing);
  }

  /** Runs a menu action and closes the menu. A failure is logged, not dropped silently. */
  function act(f: () => unknown) {
    menu = null;
    void Promise.resolve()
      .then(f)
      .catch((e) => log(`menu action: ${e}`));
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
    // A note left empty (opened with the full editor, nothing typed) is not kept as "Untitled".
    if (editing && !draft.trim() && !liveAttachments(editing).length) await invoke("note_delete", { id: editing.id });
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
    if (!standalone) void invoke("set_tab_count", { count: tabCount }).catch(() => {});
  });

  /** `saved`: the bookmark tab, which opens like the bar but on the Saved list. */
  type EdgeTarget = { kind: "bar"; saved: boolean } | { kind: "tab"; id: string };
  function targetAt(x: number, y: number): EdgeTarget | null {
    const el = (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>("[data-edge]");
    if (!el) return null;
    return el.dataset.edge === "tab" || el.dataset.edge === "peek" ? { kind: "tab", id: el.dataset.id! } : { kind: "bar", saved: el.dataset.edge === "saved" };
  }

  /** While collapsed only the main bar (and +N) opens on hover; a tab shows its title. */
  function domPointerAt(x: number, y: number, entering: boolean) {
    if (nativeCrossings && !nativeInside) return;
    pointerAt(x, y, entering);
  }

  // ---- hold a tab on the edge and move it up or down to change its place
  let tabDrag: { id: string; y: number; moved: boolean; over: { id: string; after: boolean } | null } | null = $state(null);
  let justDragged = false;

  function tabDragMove(y: number) {
    const d = tabDrag;
    if (!d) return;
    if (!d.moved && Math.abs(y - d.y) < 6) return;
    d.moved = true;
    hoverTab = null;
    const ids = edgeItems.tabs.map((n) => n.id);
    const from = ids.indexOf(d.id);
    d.over = null;
    for (const el of document.querySelectorAll<HTMLElement>(".strip .tab[data-id]")) {
      const r = el.getBoundingClientRect();
      if (el.dataset.id !== d.id && y >= r.top && y <= r.bottom) {
        // It takes the other tab's place: below it when moving down, above it when moving up.
        d.over = { id: el.dataset.id!, after: ids.indexOf(el.dataset.id!) > from };
      }
    }
  }

  function tabDragEnd() {
    const d = tabDrag;
    tabDrag = null;
    if (!d?.moved) return;
    justDragged = true; // the click that ends the hold must not open the note
    setTimeout(() => (justDragged = false), 300);
    if (!d.over) return;
    const ids = edgeItems.tabs.map((n) => n.id).filter((id) => id !== d.id);
    const before = ids[ids.indexOf(d.over.id) + (d.over.after ? 1 : 0)] ?? null;
    log(`edge tab ${d.id} moved before ${before}`);
    void movePin(d.id, before);
  }

  function pointerAt(x: number, y: number, entering: boolean) {
    if (tabDrag) {
      tabDragMove(y);
      return;
    }
    if (mode === "expanded") {
      if (entering) edge.pointerEnter();
      else edge.pointerMove();
      return;
    }
    const t = targetAt(x, y);
    setHover(t?.kind === "tab" ? t.id : null);
    if (t?.kind === "bar") {
      pendingSaved = t.saved; // which list the panel opens on, decided by where the pointer is
      if (entering) edge.pointerEnter();
      else edge.pointerMove();
    }
  }

  function openTab(n: Note) {
    pendingOpen = all.find((x) => x.id === n.id) ?? n;
    edge.click();
  }

  const hovered = $derived(edgeItems.tabs.find((n) => n.id === hoverTab) ?? null);
  // Its preview card shown for a moment counts as seen (a passing pointer does not).
  $effect(() => {
    const id = hovered?.id;
    if (!id || !unseen.has(id)) return;
    const t = setTimeout(() => seen(id), 1200);
    return () => clearTimeout(t);
  });
  // While the card shows (and the panel is collapsed), Rust adds its box to the clickable area.
  let peekEl: HTMLElement | undefined = $state();
  $effect(() => {
    const el = peekEl;
    if (standalone) return;
    if (!hovered || !el || mode === "expanded") {
      void invoke("set_peek", { rect: null }).catch(() => {});
      return;
    }
    // The layout box, not getBoundingClientRect: the card slides in from 8px to the right, and a
    // rect taken during that first frame put the Windows region 8px off, cutting the card's edge.
    void invoke("set_peek", { rect: { x: el.offsetLeft, y: el.offsetTop, w: el.offsetWidth, h: el.offsetHeight } }).catch(() => {});
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
  const deleteAccount = async () => {
    if ((await ask(`Delete the Sliptab account ${sync.email}? Everything synced to it is deleted from the server and every device is signed out. Notes on this laptop stay.`, "Delete account")) === null) return;
    void syncStep(async () => void (sync = await invoke<SyncStatus>("sync_delete_account")));
  };

  async function openSettings() {
    licensesOpen = false;
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
      else if (!standalone) edge.escape();
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
    void invoke<Settings>("get_settings").then((c) => {
      cfg = c;
      cfgLoaded = true;
    });
    if (!standalone) void listen("edge-hotkey", () => edge.hotkey());
    // An edit in the other window (edge panel or app window): reload the lists.
    void listen("notes-changed", () => void refresh());
    void invoke<SyncStatus>("sync_status").then((st) => (sync = st));
    void listen("saves-updated", () => void refresh());
    void listen<string>("update-ready", (e) => (update = { version: e.payload, ready: true }));
    void listen<{ version: string; url: string }>("update-available", (e) => (update = { version: e.payload.version, ready: false, url: e.payload.url }));
    void listen<SyncStatus>("sync-status", (e) => {
      sync = e.payload;
      if (e.payload.received > 0) {
        void refresh();
        if (!shown) arrived = true;
      }
    });
    void listen<HotkeyStatus>("edge-hotkey-status", (e) => (hotkey = e.payload));
    if (standalone && new URLSearchParams(location.search).get("open") === "first") {
      void refresh().then(() => all[0] && open(all[0])); // dev aid, see home.rs
    }
    // No browser menu (Back, Forward, Reload) on right-click: this is an app, not a web page.
    // Text fields keep theirs, for cut, copy and paste.
    document.addEventListener("contextmenu", (e) => {
      const t = e.target as HTMLElement;
      // Text: the app's own Cut / Copy / Paste menu. The system one was full of options nobody
      // needs here, and as a separate window it closed the panel under it.
      const field = t.closest<HTMLElement>("input, textarea, [contenteditable='true']");
      if (field) {
        e.preventDefault();
        menu = null;
        textMenu = { x: e.clientX, y: e.clientY, field, href: t.closest<HTMLElement>("[data-href]")?.dataset.href };
        return;
      }
      e.preventDefault();
      const id = t.closest<HTMLElement>(".panel [data-id]")?.dataset.id;
      const n = id && [...all, ...allSaves].find((x) => x.id === id);
      menu = n ? { n, x: e.clientX, y: e.clientY } : null;
    });
    // Any click, key, scroll or leaving the window closes it.
    const close = () => ((menu = null), (textMenu = null));
    document.addEventListener("pointerdown", (e) => {
      if (!(e.target as HTMLElement).closest(".attach-menu, [aria-label='Attach']")) attachMenu = false;
    });
    document.addEventListener("pointerdown", (e) => !(e.target as HTMLElement).closest(".ctx") && close());
    document.addEventListener("keydown", (e) => e.key === "Escape" && close());
    document.addEventListener("scroll", close, true);
    window.addEventListener("blur", close);
    window.addEventListener("error", (e) => log(`error: ${e.message}`));
    window.addEventListener("unhandledrejection", (e) => log(`rejection: ${e.reason}`));
    // WebKitGTK under XWayland drops DOM mouseleave; GTK's own crossings (with x/y) are reliable.
    void listen<{ kind: "enter" | "leave" | "move"; x: number; y: number }>("edge-pointer", (e) => {
      if (standalone) return;
      if (e.payload.kind === "leave") {
        onNativeLeave();
      } else {
        if (e.payload.kind === "enter") onNativeEnter();
        else lastMove = { x: e.payload.x, y: e.payload.y };
        pointerAt(e.payload.x, e.payload.y, e.payload.kind === "enter");
      }
    });
    window.addEventListener("pointerup", tabDragEnd);
    window.addEventListener("pointercancel", () => (tabDrag = null));
    document.addEventListener("mouseout", (e) => {
      if (!e.relatedTarget && !standalone) {
        hoverTab = null;
        leave();
      }
    });
    window.addEventListener("blur", () => {
      if (edge.mode === "expanded" && !standalone && !held) edge.escape();
    });
    void refresh();
  });
</script>

<svelte:window onkeydown={onKey} />

{#snippet closeButtons()}
  {#if !standalone}
    <button class="ib hold" class:on={held} onclick={() => (held = !held)} aria-pressed={held}
      aria-label={held ? "Let the panel close again" : "Keep the panel open"}
      title={held ? "Held open · click to let it close when you leave" : "Keep open while you work elsewhere"}>
      <!-- A lock (closed when held), not a pin: pin already means "tab on the edge". -->
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="11" width="14" height="10" rx="2"/>{#if held}<path d="M8 11V7a4 4 0 0 1 8 0v4"/>{:else}<path d="M8 11V7a4 4 0 0 1 7.5-1.9"/>{/if}</svg>
    </button>
    <button class="ib" onclick={openHome} aria-label="Open the Sliptab window" title="Open in a window"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/></svg></button>
    <button class="ib" onclick={() => edge.escape()} aria-label="Close panel" title="Close"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
  {/if}
{/snippet}

{#snippet header()}
  <header class="top">
    {#if standalone}<h1 class="page">{tab === "saved" ? "Saved" : "Notes"}</h1>{/if}
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
    {#if !standalone}<button class="ib" onclick={openSettings} aria-label="Settings" title="Settings"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg></button>{/if}
    {@render closeButtons()}
  </header>
  <nav class="tabs" class:hidden={standalone} aria-label="Notes or saved posts">
    <button class:on={tab === "notes"} onclick={() => showTab("notes")}>Notes</button>
    {#if savesOn}<button class:on={tab === "saved"} onclick={() => showTab("saved")}>Saved{#if allSaves.length}<span>{allSaves.length}</span>{/if}</button>{/if}
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
      <div class="hint"><span><kbd>↵</kbd> save <kbd>⇧↵</kbd> new line</span><button class="full" onclick={openFull} title="Open in the full editor: formatting and checklists">Full editor ↗</button></div>
    {/if}
  </div>
{/snippet}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<main
  class:left={cfg.side === "left"}
  class:standalone
  style:--accent={cfg.barColor ?? "#2f6b66"}
  style:--bar={cfg.barColor ?? "#8a8f99"}
  onmouseenter={(e) => {
    if (standalone) return;
    log(`mouseenter ${e.clientX},${e.clientY}`); // debug builds only: the Windows smoke test reads it
    domPointerAt(e.clientX, e.clientY, true);
  }}
  onmouseleave={() => {
    if (standalone) return;
    hoverTab = null;
    leave();
  }}
  onmousemove={(e) => !standalone && domPointerAt(e.clientX, e.clientY, false)}
>
  {#if !standalone}
  <div
    class="strip"
    ondragstart={(e) => e.preventDefault()}
    class:hidden={shown || !cfg.showBar}
    style:height="min({stripHeight}px, 100vh)"
    style:top="calc((100vh - min({stripHeight}px, 100vh)) * {cfg.barPosition})"
  >
    {#each edgeItems.tabs as n (n.id)}
      <button class="tab" class:hover={hoverTab === n.id} data-edge="tab" data-id={n.id} style:--c={n.color ?? "var(--bar)"}
        class:lifted={tabDrag?.moved && tabDrag.id === n.id}
        class:fresh={unseen.has(n.id)}
        class:drop-before={tabDrag?.over?.id === n.id && !tabDrag.over.after}
        class:drop-after={tabDrag?.over?.id === n.id && tabDrag.over.after}
        onpointerdown={(e) => { if (e.button === 0) tabDrag = { id: n.id, y: e.clientY, moved: false, over: null }; }}
        onpointermove={(e) => tabDragMove(e.clientY)}
        onclick={() => !justDragged && openTab(n)} aria-label="Open {n.title} (hold and move to change its place)"></button>
    {/each}
    {#if edgeItems.more}
      <button class="tab more" data-edge="bar" onclick={() => edge.click()} aria-label="{edgeItems.more} more pinned notes">
        <span>+{edgeItems.more}</span>
      </button>
    {/if}
    {#if today.length && savesOn}
      <button class="tab saved" data-edge="saved" onclick={() => { pendingSaved = true; edge.click(); }}
        aria-label="{today.length} saves to come back to" title="{today.length} saves to come back to today">
        <span>{today.length}</span>
      </button>
    {/if}
    <button class="bar" data-edge="bar" onclick={() => { pendingSaved = false; edge.click(); }} aria-label="Open notes ({all.length}){arrived ? ', new from another device' : ''}">
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
        {#each plain(hovered.body).split("\n").slice(1) as line}
          {@const item = /^\s*- \[([ xX])\] (.*)$/.exec(line)}
          {#if item}<div class="ci" class:done={item[1] !== " "}>{item[1] !== " " ? "☑" : "☐"} {item[2]}</div>
          {:else if line.trim()}<div>{line}</div>
          {:else}<div class="gap"></div>{/if}
        {/each}
      </div>
      <div class="peek-f">{items.length ? `${items.filter((i) => i.done).length}/${items.length} done · ` : ""}Click to edit</div>
    </div>
  {/if}

  {/if}

  {#if standalone && cfgLoaded && !cfg.onboarded}
    <Onboarding
      beta={savesOn}
      {os}
      {sync}
      hotkey={cfg.hotkey}
      side={cfg.side}
      onSignIn={signInGoogle}
      onDone={async (startAtLogin: boolean) => {
        await invoke("set_autostart", { enabled: startAtLogin }).then((v) => (autostart = v as boolean)).catch(() => {});
        await saveSettings({ onboarded: true });
      }}
    />
  {/if}

  {#if standalone}
    <nav class="rail" aria-label="Sections">
      <div class="logo" aria-hidden="true"><span></span></div>
      <button class:on={!settingsOpen && !agentsOpen && tab === "notes"} onclick={() => { settingsOpen = false; agentsOpen = false; editing = null; showTab("notes"); }} aria-label="Notes" title="Notes">
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 4h10l4 4v12H5z"/><path d="M14 4v5h5M8.5 13h7M8.5 16.5h5"/></svg>
      </button>
      {#if savesOn}
      <button class:on={!settingsOpen && !agentsOpen && tab === "saved"} onclick={() => { settingsOpen = false; agentsOpen = false; editing = null; showTab("saved"); }} aria-label="Saved" title="Saved">
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6.5 4h11v16l-5.5-4-5.5 4z"/></svg>
      </button>
      {/if}
      <button class:on={agentsOpen && !settingsOpen} onclick={() => { settingsOpen = false; editing = null; agentsOpen = true; }} aria-label="AI agents" title="AI agents (MCP)">
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6z"/><path d="M18.5 14.5l.8 2.2 2.2.8-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.8z"/></svg>
      </button>
      <span class="grow"></span>
      <button class:on={settingsOpen} onclick={() => { agentsOpen = false; openSettings(); }} aria-label="Settings" title="Settings">
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg>
      </button>
    </nav>
  {/if}

  {#if (update || tooOld) && (standalone || shown)}
    <div class="update" class:urgent={tooOld}>
      <span>
        {#if update?.ready}Sliptab {update.version} is installed.
        {:else if update}Sliptab {update.version} is available.
        {:else}This version of Sliptab can no longer sync. Updating…{/if}
      </span>
      {#if update?.ready}
        <button onclick={() => invoke("restart_for_update")}>Restart</button>
      {:else if update?.url}
        <button onclick={() => invoke("open_link", { url: update!.url })}>Download</button>
      {/if}
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="panel" class:shown style:--open-ms="{OPEN_MS}ms" style:--close-ms="{CLOSE_MS}ms" onpointerdown={claimFocus}>

    {#if agentsOpen && !settingsOpen && standalone}
      <Agents access={cfg.mcpAccess} onaccess={(a) => saveSettings({ mcpAccess: a })} />
    {:else if settingsOpen && licensesOpen}
      <header class="top">
        <button class="ib" onclick={() => (licensesOpen = false)} aria-label="Back to settings"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 18l-6-6 6-6"/></svg></button>
        <h1 class="grow">Open-source licences</h1>
        {@render closeButtons()}
      </header>
      <Licenses />
    {:else if settingsOpen}
      <header class="top">
        <button class="ib" onclick={() => (settingsOpen = false)} aria-label="Back to notes"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 18-6-6 6-6"/></svg></button>
        <h1 class="grow">Settings</h1>
        {@render closeButtons()}
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
            <div class="grow"><div>Plan</div><div class="sub">{plan.text}</div></div>
            {#if plan.paid}
              <button class="pill" onclick={() => billing("portal")}>Manage</button>
            {:else if sync.billing}
              <button class="pill" onclick={() => billing("monthly")}>Monthly</button>
              <button class="pill" onclick={() => billing("yearly")}>Yearly</button>
            {/if}
          </section>
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
            <div class="sub">Get your phone's notes here. Free for 30 days, no card needed.</div>
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

        <div class="group">Sliptab</div>
        <section class="setting">
          <div class="grow">Screen edge</div>
          <div class="seg" role="group" aria-label="Screen edge">
            <button class:on={cfg.side === "left"} onclick={() => saveSettings({ side: "left" })}>Left</button>
            <button class:on={cfg.side === "right"} onclick={() => saveSettings({ side: "right" })}>Right</button>
          </div>
        </section>
        <section class="setting barpos">
          <div class="grow">
            <label for="barpos">Bar position</label>
            <input id="barpos" type="range" min="0" max="1" step="0.01" value={cfg.barPosition}
              oninput={(e) => (barPreview = +e.currentTarget.value)}
              onchange={(e) => { barPreview = null; void saveSettings({ barPosition: +e.currentTarget.value }); }} />
          </div>
          <!-- The panel covers the real bar while settings are open, so this small screen shows
               where the bar and its tabs will sit. -->
          <div class="screen" class:left={cfg.side === "left"} aria-hidden="true">
            <div class="mini" style:top="calc((100% - {12 + edgeItems.tabs.length * 7}px - 26px) * {barPreview ?? cfg.barPosition} + 6px)">
              {#each edgeItems.tabs as n (n.id)}<i style:background={n.color ?? "var(--faint)"}></i>{/each}
              <b></b>
            </div>
          </div>
        </section>
        <section class="setting">
          <div class="grow"><div>Open on hover</div><div class="sub">Off: opens only on click or the hotkey.</div></div>
          <input type="checkbox" checked={cfg.hoverOpen} onchange={(e) => saveSettings({ hoverOpen: e.currentTarget.checked })} aria-label="Open on hover" />
        </section>
        <section class="setting">
          <div class="grow"><div>Show the bar</div><div class="sub">Off: nothing on screen, only the hotkey opens notes.</div></div>
          <input type="checkbox" checked={cfg.showBar} onchange={(e) => saveSettings({ showBar: e.currentTarget.checked })} aria-label="Show the bar" />
        </section>

        <div class="group">AI agents</div>
        <section class="setting">
          <div class="grow"><div>Agent access</div><div class="sub">Set up in the Sliptab window → AI agents.</div></div>
          <select value={cfg.mcpAccess} onchange={(e) => saveSettings({ mcpAccess: e.currentTarget.value as Settings["mcpAccess"] })} aria-label="Agent access">
            <option value="off">Off</option>
            <option value="read">Read</option>
            <option value="write">Read & write</option>
            <option value="full">Full access</option>
          </select>
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

        <div class="group">About</div>
        <section class="setting">
          <div class="grow"><div>Sliptab {appVersion}</div><div class="sub">Open-source packages Sliptab is built with</div></div>
          <button class="pill" onclick={() => (licensesOpen = true)}>Licences</button>
        </section>
      </div>
    {:else if editing}
      <header class="top">
        <button class="ib" onclick={back} aria-label="Back to notes"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 18-6-6 6-6"/></svg></button>
        <h1 class="grow ellipsis">{editing.title || "Untitled"}</h1>
        <button class="ib" class:pinned={editing.pinned} style:--c={editing.color ?? "var(--faint)"} onclick={() => editing && togglePin(editing)}
          aria-label={editing.pinned ? "Unpin" : "Pin to the edge"} title={editing.pinned ? "Unpin" : "Pin to the edge"}><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 17v5"/><path d="M9 3h6l-1 7 4 3v2H6v-2l4-3z"/></svg></button>
        {@render closeButtons()}
      </header>
      {#if editing.pinned}
        <div class="swatches tabcolor" aria-label="Tab colour">
          {#each PALETTE as c}
            <button class="sw small" class:on={editing.color === c} style:--c={c} onclick={() => editing && setColor(editing, c)} aria-label="Tab colour {c}"></button>
          {/each}
        </div>
      {/if}
      <div class="format-bar" role="toolbar" aria-label="Formatting">
        <button onclick={() => rich?.bold()} title="Bold (Ctrl+B)" aria-label="Bold"><b>B</b></button>
        <button onclick={() => rich?.italic()} title="Italic (Ctrl+I)" aria-label="Italic"><i>I</i></button>
        <button onclick={() => rich?.underline()} title="Underline (Ctrl+U)" aria-label="Underline"><u>U</u></button>
        <button onclick={() => rich?.strike()} title="Strikethrough (Ctrl+Shift+X)" aria-label="Strikethrough"><s>S</s></button>
        <button onclick={() => rich?.checklist()} title="Checklist (Ctrl+Shift+L)" aria-label="Checklist"><svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="4"/><path d="M8 12.5l2.8 2.8L16 9.5"/></svg></button>
        <button onclick={() => (attachMenu = !attachMenu)} title="Attach an image or a file (or paste / drop one)" aria-label="Attach"><svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.4 11.1l-9.2 9.2a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"/></svg></button>
        {#if attachMenu}
          <div class="ctx attach-menu" role="menu">
            <button role="menuitem" onclick={() => { attachMenu = false; galleryInput?.click(); }}>Gallery <span>images</span></button>
            <button role="menuitem" onclick={() => { attachMenu = false; fileInput?.click(); }}>Document <span>any file, music, recordings</span></button>
          </div>
        {/if}
        <input bind:this={galleryInput} type="file" accept="image/*" multiple hidden onchange={(e) => { const t = e.currentTarget; void attach([...(t.files ?? [])]); t.value = ""; }} />
        <input bind:this={fileInput} type="file" multiple hidden onchange={(e) => { const t = e.currentTarget; void attach([...(t.files ?? [])]); t.value = ""; }} />
        <span class="fb-hint">Select text, then B / I / U / S · hover a link to open or copy it</span>
      </div>
      {#if attaching}<div class="attaching">{attaching}</div>{/if}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="editor" onmousedown={(e) => {
        // The text is only as tall as its lines: a click on the empty space below types at the end.
        if (!(e.target as HTMLElement).closest(".rich, .atts")) {
          e.preventDefault();
          rich?.focusEnd();
        }
      }}>
        <Attachments items={liveAttachments(editing)} onremove={detach} />
        <RichEditor bind:this={rich} bind:el={editorEl} bind:value={draft} oninput={scheduleSave} onfiles={attach} onopenlink={(href) => void invoke("open_link", { url: href })} />
      </div>
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
          <div class="empty">{query ? "No matches" : "Share a post to Sliptab on your phone, or paste a link below. Saves land here."}</div>
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
        onMovePin={movePin}
        {unseen}
        onHover={(n: Note) => (selectedId = n.id)}
      />
    {/if}
  </div>

  {#if asking}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="ask-back" onclick={() => answer(null)}>
      <div class="ask" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()}>
        <p>{asking.text}</p>
        {#if asking.input !== null}
          <!-- svelte-ignore a11y_autofocus -->
          <input autofocus bind:value={asking.input} onkeydown={(e) => e.key === "Enter" && answer(asking!.input)} />
        {/if}
        <div class="row">
          <button onclick={() => answer(null)}>Cancel</button>
          <!-- svelte-ignore a11y_autofocus -->
          <button class="go" class:danger={asking.ok.startsWith("Delete")} autofocus={asking.input === null} onclick={() => answer(asking!.input ?? "")}>{asking.ok}</button>
        </div>
      </div>
    </div>
  {/if}

  {#if textMenu}
    {@const f = textMenu.field}
    {@const has = !!fieldSelection(f)}
    <!-- mousedown is cancelled so the field keeps its focus and selection for Cut / Copy. -->
    <div class="ctx" role="menu" tabindex="-1" onmousedown={(e) => e.preventDefault()} style:left="min({textMenu.x}px, calc(100vw - 210px))" style:top="min({textMenu.y}px, calc(100vh - 330px))">
      {#if textMenu.href}
        {@const href = textMenu.href}
        <button role="menuitem" onclick={() => { textMenu = null; void invoke("open_link", { url: href }); }}>Open link</button>
        <button role="menuitem" onclick={() => { textMenu = null; void clipWrite(href); }}>Copy link</button>
        <hr />
      {/if}
      <button role="menuitem" disabled={!has} onclick={() => textAction("cut")}>Cut <kbd>Ctrl X</kbd></button>
      <button role="menuitem" disabled={!has} onclick={() => textAction("copy")}>Copy <kbd>Ctrl C</kbd></button>
      <button role="menuitem" onclick={() => textAction("paste")}>Paste <kbd>Ctrl V</kbd></button>
      <button role="menuitem" onclick={() => textAction("all")}>Select all <kbd>Ctrl A</kbd></button>
      {#if inEditor(f)}
        <hr />
        <button role="menuitem" onclick={() => { textMenu = null; rich?.bold(); }}>Bold <kbd>Ctrl B</kbd></button>
        <button role="menuitem" onclick={() => { textMenu = null; rich?.italic(); }}>Italic <kbd>Ctrl I</kbd></button>
        <button role="menuitem" onclick={() => { textMenu = null; rich?.underline(); }}>Underline <kbd>Ctrl U</kbd></button>
        <button role="menuitem" onclick={() => { textMenu = null; rich?.strike(); }}>Strikethrough</button>
        <button role="menuitem" onclick={() => { textMenu = null; rich?.checklist(); }}>Checklist</button>
      {/if}
    </div>
  {/if}

  {#if menu}
    {@const n = menu.n}
    <div class="ctx" role="menu" style:left="min({menu.x}px, calc(100vw - 210px))" style:top="min({menu.y}px, calc(100vh - 190px))">
      {#if n.link}
        <button role="menuitem" onclick={() => act(() => openSave(n))}>Open {platformName(n.link)}</button>
        <button role="menuitem" onclick={() => act(() => open(n))}>Edit note</button>
        <button role="menuitem" onclick={() => act(() => markDone(n))}>{n.link.done ? "Not done" : "Mark done"}</button>
        <button role="menuitem" onclick={() => act(() => navigator.clipboard.writeText(n.link!.url))}>Copy link</button>
      {:else}
        <button role="menuitem" onclick={() => act(() => open(n))}>Open</button>
        <button role="menuitem" onclick={() => act(() => togglePin(n))}>{n.pinned ? "Unpin" : "Pin to the edge"}</button>
        <button role="menuitem" onclick={() => act(() => navigator.clipboard.writeText(n.body))}>Copy text</button>
      {/if}
      <hr />
      <button role="menuitem" class="danger" onclick={() => act(() => (n.link ? deleteSave(n) : deleteNote(n)))}>Delete</button>
    </div>
  {/if}
</main>

<style>
  .pill.danger { color: #e5484d; }
  .update {
    position: fixed;
    z-index: 55;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 8px 8px 14px;
    border-radius: 12px;
    background: var(--text);
    color: var(--bg);
    font-size: 13px;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.2);
    white-space: nowrap;
  }
  .update.urgent {
    background: #b4232a;
    color: #fff;
  }
  .update button {
    all: unset;
    padding: 5px 12px;
    border-radius: 8px;
    background: var(--bg);
    color: var(--text);
    font-weight: 600;
    cursor: pointer;
  }
  /* Dragging a pinned note (lib/pindrag.ts): where it will land. */
  :global([data-pin].dragging) {
    opacity: 0.45;
  }
  :global([data-pin].drop-before) {
    box-shadow: -3px 0 0 var(--accent);
  }
  :global([data-pin].drop-after) {
    box-shadow: 3px 0 0 var(--accent);
  }
  :global(.list [data-pin].drop-before) {
    box-shadow: 0 -2px 0 var(--accent);
  }
  :global(.list [data-pin].drop-after) {
    box-shadow: 0 2px 0 var(--accent);
  }
  .setting.barpos input {
    width: 100%;
    margin-top: 8px;
  }
  .screen {
    position: relative;
    flex: none;
    width: 92px;
    height: 58px;
    border-radius: 6px;
    border: 1.5px solid var(--line);
    background: var(--surface-2);
  }
  .screen .mini {
    position: absolute;
    right: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
  }
  .screen.left .mini {
    right: auto;
    left: 0;
    align-items: flex-start;
  }
  .screen .mini i {
    width: 7px;
    height: 5px;
    border-radius: 2px 0 0 2px;
  }
  .screen.left .mini i {
    border-radius: 0 2px 2px 0;
  }
  .screen .mini b {
    width: 3px;
    height: 20px;
    border-radius: 2px 0 0 2px;
    background: var(--bar); /* the bar colour from settings */
  }
  .ib.hold.on {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .ask-back {
    position: fixed;
    inset: 0;
    z-index: 70;
    background: rgba(0, 0, 0, 0.35);
    display: grid;
    grid-template-columns: minmax(0, 320px); /* the track never grows with its content */
    justify-content: center;
    align-content: center;
    place-items: center;
    padding: 16px;
  }
  .ask {
    width: min(320px, 100%);
    padding: 16px;
    border-radius: 14px;
    background: var(--surface);
    border: 1px solid var(--line);
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.25);
    font-size: 13.5px;
    color: var(--text);
  }
  .ask p {
    margin: 0 0 12px;
    line-height: 1.45;
    overflow-wrap: anywhere; /* a note titled by a long link must not widen the dialog */
  }
  .ask input {
    width: 100%;
    box-sizing: border-box;
    margin-bottom: 12px;
    padding: 7px 9px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: var(--bg);
    color: var(--text);
    font: inherit;
  }
  .ask .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .ask button {
    all: unset;
    cursor: pointer;
    padding: 6px 12px;
    border-radius: 8px;
    background: var(--surface-2);
  }
  .ask .go {
    background: var(--text);
    color: var(--bg);
  }
  .ask .go.danger {
    background: #e5484d;
    color: #fff;
  }
  .ctx {
    position: fixed;
    z-index: 60;
    min-width: 190px;
    padding: 5px;
    border-radius: 11px;
    background: var(--surface);
    border: 1px solid var(--line);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.18);
    display: flex;
    flex-direction: column;
  }
  .ctx button {
    all: unset;
    padding: 7px 11px;
    border-radius: 7px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
  }
  .ctx button:hover,
  .ctx button:focus-visible {
    background: var(--surface-2);
  }
  .ctx button:disabled {
    color: var(--faint);
    cursor: default;
    background: none;
  }
  .ctx kbd {
    float: right;
    margin-left: 18px;
    font: inherit;
    font-size: 11.5px;
    color: var(--faint);
  }
  .ctx .danger {
    color: #e5484d;
  }
  .ctx hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 4px 2px;
  }
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
    /* Warm paper tones (Wispr Flow-like): off-white, thin warm borders, ink text. */
    --bg: #f6f4f0;
    --surface: #fcfbf9;
    --surface-2: #efece6;
    --text: #1c1c1a;
    --muted: #6b6862;
    --faint: #a19d95;
    --line: #e6e2da;
    --shadow: rgba(40, 30, 10, 0.06);
    --serif: "Instrument Serif", Georgia, serif;
    font-family: "Figtree Variable", system-ui, sans-serif;
  }
  :global(:root[data-theme="dark"]) {
    --bg: #171614;
    --surface: #1f1e1b;
    --surface-2: #292725;
    --text: #ecebe7;
    --muted: #a7a39b;
    --faint: #706c65;
    --line: #34312d;
    --shadow: rgba(0, 0, 0, 0.45);
  }
  @media (prefers-color-scheme: dark) {
    :global(:root[data-theme="system"]) {
      --bg: #171614;
      --surface: #1f1e1b;
      --surface-2: #292725;
      --text: #ecebe7;
      --muted: #a7a39b;
      --faint: #706c65;
      --line: #34312d;
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
  /* An app, not a page: lists, cards and labels are not selectable text (a press-and-drag
     selected words); the editor, fields and previews still are. */
  /* No scrollbars anywhere (they looked heavy on GNOME); lists still scroll by wheel, trackpad
     and keys. */
  :global(*) {
    scrollbar-width: none;
  }
  :global(*::-webkit-scrollbar) {
    display: none;
  }
  /* The notes list keeps one: a hairline thumb with no track, so a long list shows how much
     there is. It darkens while the pointer is over the list. */
  :global(.notes-scroll) {
    /* The bar sits in a gutter outside the cards (the cards keep their width, lined up with the
       box above), so it never draws over them. */
    /* WebKitGTK draws overlay scrollbars that take no room: reserve a 12px gutter by padding,
       pulled back out with a negative margin so the cards stay aligned with the box above. */
    margin-right: -12px !important;
    padding-right: 12px !important;
    scrollbar-width: thin;
    scrollbar-color: color-mix(in srgb, var(--text) 14%, transparent) transparent;
  }
  :global(.notes-scroll::-webkit-scrollbar) {
    display: block;
    width: 10px;
  }
  :global(.notes-scroll::-webkit-scrollbar-track) {
    background: transparent;
  }
  :global(.notes-scroll::-webkit-scrollbar-thumb) {
    border-radius: 99px;
    border: 3.5px solid transparent;
    background-clip: padding-box;
    background-color: color-mix(in srgb, var(--text) 12%, transparent);
    min-height: 40px;
  }
  :global(.notes-scroll:hover::-webkit-scrollbar-thumb) {
    background-color: color-mix(in srgb, var(--text) 26%, transparent);
  }
  main {
    user-select: none;
    -webkit-user-select: none;
  }
  main :global(input),
  main :global(textarea),
  main :global(.rich),
  main :global([contenteditable]) {
    user-select: text;
    -webkit-user-select: text;
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
  /* Pinned on the other device and not seen here: the tab flashes bright with a light running
     down it (the edge window is only the strip's width, so nothing may move outside it) until
     its card is looked at or the note is opened. */
  .tab.fresh {
    background-image: linear-gradient(180deg, transparent 0%, rgba(255, 255, 255, 0.85) 50%, transparent 100%);
    background-size: 100% 60%;
    background-repeat: no-repeat;
    animation: fresh 1.4s ease-in-out infinite;
  }
  @keyframes fresh {
    0% { background-position: 0 -120%; filter: brightness(1); }
    50% { filter: brightness(1.45) saturate(1.3); }
    100% { background-position: 0 220%; filter: brightness(1); }
  }
  @media (prefers-reduced-motion: reduce) {
    .tab.fresh { animation: none; filter: brightness(1.4); }
  }
  .tab.lifted {
    opacity: 0.5;
  }
  .tab.drop-before {
    box-shadow: 0 -3px 0 var(--text);
  }
  .tab.drop-after {
    box-shadow: 0 3px 0 var(--text);
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

  /* ---- the app window: an icon rail and the content on an inset paper surface */
  main.standalone {
    flex-direction: row;
    background: var(--bg);
    padding: 10px 10px 10px 0;
    box-sizing: border-box;
  }
  .rail {
    width: 64px;
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 8px 0 6px;
  }
  .rail .logo {
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background: var(--text);
    display: grid;
    place-items: center;
    margin-bottom: 22px;
  }
  .rail .logo span {
    width: 4px;
    height: 16px;
    border-radius: 3px;
    background: var(--bg);
    transform: translateX(6px);
  }
  .rail button {
    width: 40px;
    height: 40px;
    border-radius: 11px;
    display: grid;
    place-items: center;
    background: none;
    color: var(--muted);
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }
  .rail button:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .rail button.on {
    background: var(--surface-2);
    color: var(--text);
  }
  .rail .grow {
    flex: 1;
  }
  main.standalone .panel {
    min-width: 0; /* a flex item's default min-width is its content: a long line widened it past the window */
    transform: none;
    opacity: 1;
    transition: none;
    border-radius: 16px;
    border: 1px solid var(--line);
    box-shadow: 0 1px 2px var(--shadow);
    background: var(--surface);
    padding: 34px max(32px, calc((100% - 760px) / 2)) 18px;
    gap: 14px;
  }
  main.standalone :global(.top) {
    gap: 14px;
    margin-bottom: 6px;
  }
  .page {
    font-size: 24px;
    font-weight: 600;
    letter-spacing: -0.01em;
    margin: 0 auto 0 0;
    color: var(--text);
  }
  main.standalone .search {
    max-width: 280px;
  }
  .tabs.hidden {
    display: none;
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
    min-width: 0; /* shrink with the panel: an input's own width overflowed a narrow header */
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
  .hint .full {
    all: unset;
    cursor: pointer;
    color: var(--muted);
    padding: 0 2px;
    border-radius: 4px;
  }
  .hint .full:hover {
    color: var(--text);
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
  .format-bar {
    position: relative;
  }
  .attach-menu {
    position: absolute;
    top: 32px;
    left: 150px;
  }
  .attach-menu span {
    color: var(--faint);
    font-size: 11.5px;
    margin-left: 6px;
  }
  .attaching {
    font-size: 12px;
    color: var(--muted);
    padding: 0 4px;
  }
  .format-bar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
  }
  .format-bar button {
    width: 30px;
    height: 28px;
    border-radius: 7px;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 14px;
    cursor: pointer;
  }
  .format-bar button:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .fb-hint {
    margin-left: auto;
    font-size: 11.5px;
    color: var(--faint);
  }
  .editor {
    flex: 1;
    min-height: 0;
    overflow: auto;
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
