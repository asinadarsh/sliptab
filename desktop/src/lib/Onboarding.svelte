<!-- First-run tour in the app window, laid out like Wispr Flow's: a step list across the top, the
     action on the left, a warm gradient with a serif line and a picture of the product on the
     right. The two "try it" steps tick themselves when the user really opens the edge and presses
     the shortcut. -->
<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  let {
    os,
    beta = false,
    sync,
    hotkey,
    side,
    onSignIn,
    onDone,
  }: {
    os: "macos" | "windows" | "linux";
    /** Beta accounts also hear about Saved posts. */
    beta?: boolean;
    sync: { email: string | null; error: string | null };
    hotkey: string;
    side: "left" | "right";
    onSignIn: () => Promise<void> | void;
    onDone: (startAtLogin: boolean) => Promise<void>;
  } = $props();

  const STEPS = ["Welcome", "Your edge", "Shortcut", "Phone", "Ready"];
  let step = $state(Math.min(4, Number(new URLSearchParams(location.search).get("step")) || 0)); // ?step= only in dev builds
  let back = $state(false);
  let opened = $state(false);
  let pressed = $state(false);
  let startAtLogin = $state(true);
  let finishing = $state(false);
  let leaving = $state(false);
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const ms = (n: number) => (reduced ? 0 : n);

  const names: Record<string, Record<string, string>> = {
    macos: { Super: "⌘", Control: "⌃", Alt: "⌥", Shift: "⇧" },
    windows: { Super: "Win", Control: "Ctrl" },
    linux: { Super: "Super", Control: "Ctrl" },
  };
  const keys = $derived(hotkey.split("+").map((k) => names[os][k] ?? k));

  function go(to: number) {
    back = to < step;
    step = Math.max(0, Math.min(STEPS.length - 1, to));
  }
  // A "try it" step moves on by itself shortly after it worked, so the tick is seen first.
  const autoNext = (from: number) => setTimeout(() => step === from && go(from + 1), ms(1100));

  onMount(() => {
    const off = [
      listen("edge-opened", () => {
        if (opened) return;
        opened = true;
        if (step === 1) autoNext(1);
      }),
      listen("edge-hotkey", () => {
        if (pressed) return;
        pressed = true;
        if (step === 2) autoNext(2);
      }),
    ];
    return () => off.forEach((p) => void p.then((f) => f()));
  });

  async function finish() {
    finishing = true;
    await onDone(startAtLogin);
    leaving = true;
  }

  const slide = () => ({ y: back ? -10 : 10, duration: ms(320), easing: cubicOut, opacity: 0 });
</script>

{#if !leaving}
  <div class="tour" out:fade={{ duration: ms(260) }} role="dialog" aria-modal="true" aria-label="Welcome to Sliptab">
    <ol class="steps">
      {#each STEPS as s, i}
        <li class:on={i === step} class:past={i < step}>
          {#if i < step}<button onclick={() => go(i)}>{s}</button>{:else}{s}{/if}
        </li>
        {#if i < STEPS.length - 1}<li class="sep" aria-hidden="true">›</li>{/if}
      {/each}
    </ol>

    <div class="split">
      <!-- left: what to do -->
      <div class="left">
        <div class="brand"><span class="logo" aria-hidden="true"><span></span></span>Sliptab</div>
        {#key step}
          <div class="copy" in:fly={slide()}>
            {#if step === 0}
              <h1>Let's get you started</h1>
              <p>{beta ? "Notes and saved posts, right on the edge of your screen." : "Quick notes, right on the edge of your screen."}</p>
              <button class="primary" onclick={() => go(1)}>Get started <span aria-hidden="true">→</span></button>
            {:else if step === 1}
              <h1>Meet your edge</h1>
              <p>Move your mouse onto the thin bar at the {side} edge of your screen. The panel slides out.</p>
              <div class="status" class:ok={opened}>
                {#if opened}<span class="tick">✓</span> Opened. That's all it takes.{:else}<span class="pulse"></span> Waiting for you to try it{/if}
              </div>
              <button class="primary" onclick={() => go(2)}>{opened ? "Continue" : "Skip for now"}</button>
            {:else if step === 2}
              <h1>One shortcut, from any app</h1>
              <p>Press <b>{keys.join(" + ")}</b> now. Sliptab opens ready to type. Press it again to close.</p>
              <div class="status" class:ok={pressed}>
                {#if pressed}<span class="tick">✓</span> Works everywhere.{:else}<span class="pulse"></span> Waiting for the shortcut{/if}
              </div>
              <button class="primary" onclick={() => go(3)}>{pressed ? "Continue" : "Skip for now"}</button>
              <span class="fine">You can change it any time in Settings.</span>
            {:else if step === 3}
              <h1>Bring your phone along</h1>
              {#if sync.email}
                <p>Signed in as <b>{sync.email}</b>. {beta ? "Notes and saves sync both ways." : "Your notes sync both ways."}</p>
                <button class="primary" onclick={() => go(4)}>Continue</button>
              {:else}
                <p>Sign in with the Google account you use in Sliptab on your phone. Free for 30 days, no card.</p>
                <button class="primary" onclick={() => onSignIn()}>
                  <svg viewBox="0 0 48 48" width="16" height="16" aria-hidden="true"><path fill="#FFC107" d="M43.6 20.5H42V20H24v8h11.3C33.7 32.7 29.2 36 24 36c-6.6 0-12-5.4-12-12s5.4-12 12-12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 12.9 4 4 12.9 4 24s8.9 20 20 20 20-8.9 20-20c0-1.3-.1-2.4-.4-3.5z"/><path fill="#FF3D00" d="m6.3 14.7 6.6 4.8C14.7 15.1 19 12 24 12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 16.3 4 9.7 8.3 6.3 14.7z"/><path fill="#4CAF50" d="M24 44c5.2 0 9.9-2 13.4-5.2l-6.2-5.2C29.2 35.1 26.7 36 24 36c-5.2 0-9.6-3.3-11.3-8l-6.5 5C9.5 39.6 16.2 44 24 44z"/><path fill="#1976D2" d="M43.6 20.5H42V20H24v8h11.3c-.8 2.2-2.2 4.2-4.1 5.6l6.2 5.2C37 39.2 44 34 44 24c0-1.3-.1-2.4-.4-3.5z"/></svg>
                  Continue with Google
                </button>
                {#if sync.error}<span class="fine err">{sync.error}</span>{/if}
                <button class="link" onclick={() => go(4)}>Not now</button>
              {/if}
            {:else}
              <h1>You're all set</h1>
              <p>Sliptab lives on your screen edge. This window is here whenever you want it: the Start menu, the tray icon, or the window button in the panel.</p>
              <label class="switch">
                <input type="checkbox" bind:checked={startAtLogin} />
                <span class="track"><span class="thumb"></span></span>
                Start Sliptab when I log in
              </label>
              <button class="primary" disabled={finishing} onclick={finish}>Open Sliptab</button>
            {/if}
          </div>
        {/key}
        {#if step > 0 && step < STEPS.length - 1}
          <button class="back" onclick={() => go(step - 1)}>← Back</button>
        {/if}
      </div>

      <!-- right: the warm side with a serif line and a picture of the product -->
      <div class="right" aria-hidden="true">
        {#key step}
          <div class="scene" in:fade={{ duration: ms(380) }}>
            {#if step === 0}
              <h2>Notes <em>on the edge</em> of your screen.</h2>
              <div class="desk">
                <div class="win"><span></span><span></span><span></span></div>
                <div class="sheet">
                  <div class="note"><b>Call Ravi about the venue</b><i></i></div>
                  <div class="note"><b>Groceries</b><i class="short"></i></div>
                  {#if beta}<div class="note"><b>Saved · 5 websites every builder…</b><i></i></div>{:else}<div class="note"><b>Ideas for the weekend</b><i></i></div>{/if}
                </div>
              </div>
            {:else if step === 1}
              <h2>Just <em>move</em> to the edge.</h2>
              <div class="screen" class:left={side === "left"} class:done={opened}>
                <div class="bar"></div>
                <div class="panel"><i></i><i class="short"></i><i></i></div>
                <div class="cursor"></div>
              </div>
            {:else if step === 2}
              <h2>Open it from <em>anywhere</em>.</h2>
              <div class="keys" class:done={pressed}>
                {#each keys as k, i}{#if i}<span class="plus">+</span>{/if}<kbd>{k}</kbd>{/each}
              </div>
            {:else if step === 3}
              <h2>Your phone, <em>in sync</em>.</h2>
              <div class="devices">
                <div class="laptop"><i></i><i class="short"></i><i></i></div>
                <div class="phone"><span class="notch"></span><i></i><i class="short"></i><i></i></div>
              </div>
            {:else if beta}
              <h2>Saved posts <em>come back</em>.</h2>
              <div class="today">
                <div class="label">For today</div>
                <div class="row"><span class="dot ig"></span>Instagram reel by Hanjla Ali</div>
                <div class="row"><span class="dot th"></span>5 Websites Every Vibecoder Should Know</div>
                <div class="row"><span class="dot x"></span>How many posts do you publish on X?</div>
              </div>
            {:else}
              <h2>Pinned notes <em>stay in sight</em>.</h2>
              <div class="today">
                <div class="label">Pinned to the edge</div>
                <div class="row"><span class="dot" style="background:#f5a524"></span>Call Ravi about the venue</div>
                <div class="row"><span class="dot" style="background:#3b82f6"></span>Groceries</div>
                <div class="row"><span class="dot" style="background:#22c55e"></span>Ideas for the weekend</div>
              </div>
            {/if}
          </div>
        {/key}
      </div>
    </div>
  </div>
{/if}

<style>
  .tour {
    /* Always the light paper palette, like Wispr Flow's tour, whatever the system theme. */
    --surface: #fcfbf9;
    --surface-2: #f1eee8;
    --text: #1c1c1a;
    --muted: #6b6862;
    --faint: #a19d95;
    --line: #e9e5de;
    color-scheme: light;
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    color: var(--text);
  }

  /* step list across the top */
  .steps {
    list-style: none;
    margin: 0;
    padding: 14px 24px;
    display: flex;
    justify-content: center;
    gap: 22px;
    border-bottom: 1px solid var(--line);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--faint);
  }
  .steps li.on {
    color: var(--text);
  }
  .steps li.past,
  .steps li.past button {
    color: var(--muted);
  }
  .steps button {
    all: unset;
    cursor: pointer;
  }
  .steps .sep {
    letter-spacing: 0;
  }

  .split {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(340px, 0.85fr) 1.15fr;
  }

  /* left: the action */
  .left {
    position: relative;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 0 clamp(32px, 6vw, 84px);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 19px;
    font-weight: 650;
    letter-spacing: -0.01em;
    margin-bottom: 26px;
  }
  .logo {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    background: var(--text);
    display: grid;
    place-items: center;
  }
  .logo span {
    width: 3px;
    height: 12px;
    border-radius: 2px;
    background: var(--surface);
    transform: translateX(5px);
  }
  .copy {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 14px;
    max-width: 380px;
  }
  h1 {
    margin: 0;
    font-size: 30px;
    font-weight: 550;
    letter-spacing: -0.02em;
    line-height: 1.15;
  }
  p {
    margin: 0;
    font-size: 15px;
    line-height: 1.55;
    color: var(--muted);
  }
  p b {
    color: var(--text);
    font-weight: 600;
  }
  .primary {
    margin-top: 10px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
    min-width: 230px;
    padding: 12px 22px;
    border-radius: 10px;
    border: none;
    background: var(--text);
    color: var(--surface);
    font: inherit;
    font-size: 14.5px;
    font-weight: 600;
    cursor: pointer;
    transition: transform 0.15s, opacity 0.15s;
  }
  .primary:hover {
    opacity: 0.9;
  }
  .primary:active {
    transform: scale(0.985);
  }
  .primary:disabled {
    opacity: 0.55;
  }
  .link,
  .back {
    all: unset;
    cursor: pointer;
    font-size: 13.5px;
    color: var(--muted);
  }
  .link:hover,
  .back:hover {
    color: var(--text);
  }
  .back {
    position: absolute;
    left: clamp(32px, 6vw, 84px);
    bottom: 26px;
  }
  .fine {
    font-size: 12.5px;
    color: var(--faint);
  }
  .err {
    color: #d14343;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 13.5px;
    color: var(--muted);
    padding: 9px 13px;
    border-radius: 9px;
    background: var(--surface-2);
  }
  .status.ok {
    color: #1f6f4a;
    background: color-mix(in srgb, #22c55e 13%, var(--surface));
  }
  .tick {
    font-weight: 700;
  }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.4s ease-in-out infinite;
  }

  .switch {
    display: flex;
    align-items: center;
    gap: 11px;
    font-size: 14px;
    cursor: pointer;
    margin-top: 4px;
  }
  .switch input {
    position: absolute;
    opacity: 0;
  }
  .track {
    width: 38px;
    height: 22px;
    border-radius: 999px;
    background: var(--line);
    position: relative;
    transition: background 0.2s;
  }
  .thumb {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
    transition: transform 0.22s cubic-bezier(0.16, 1, 0.3, 1);
  }
  .switch input:checked + .track {
    background: var(--accent);
  }
  .switch input:checked + .track .thumb {
    transform: translateX(16px);
  }
  .switch input:focus-visible + .track {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  /* right: warm gradient, serif line, product picture */
  .right {
    position: relative;
    overflow: hidden;
    background:
      radial-gradient(120% 90% at 100% 0%, #f9c7a5 0%, transparent 55%),
      radial-gradient(90% 80% at 10% 100%, #f3d4e4 0%, transparent 60%),
      linear-gradient(160deg, #fbf3e6 0%, #f8ece0 100%);
  }
  .scene {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: 13vh;
    gap: 7vh;
    color: #1c1c1a; /* the warm side stays light in dark mode, like a photograph */
  }
  h2 {
    margin: 0;
    font-family: var(--serif);
    font-weight: 400;
    font-size: clamp(30px, 3.6vw, 46px);
    letter-spacing: -0.01em;
    text-align: center;
    padding: 0 24px;
  }
  h2 em {
    font-style: italic;
  }

  .desk,
  .screen,
  .today,
  .laptop {
    background: #fffdfa;
    border-radius: 14px;
    box-shadow: 0 18px 50px rgba(120, 70, 30, 0.16), 0 0 0 1px rgba(60, 40, 20, 0.06);
  }
  .desk {
    width: min(78%, 420px);
    height: 250px;
    position: relative;
    overflow: hidden;
  }
  .win {
    display: flex;
    gap: 6px;
    padding: 12px 14px;
  }
  .win span {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #e8e2d8;
  }
  .sheet {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 48%;
    background: #f6f3ee;
    border-left: 1px solid #ece6dc;
    padding: 16px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    animation: slidein 0.7s cubic-bezier(0.16, 1, 0.3, 1) both 0.15s;
  }
  .note {
    background: #fffdfa;
    border-radius: 9px;
    padding: 9px 10px;
    font-size: 11.5px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .note b {
    font-weight: 600;
  }
  i {
    display: block;
    height: 6px;
    border-radius: 3px;
    background: #ece7de;
  }
  i.short {
    width: 55%;
  }

  .screen {
    position: relative;
    width: min(78%, 420px);
    height: 250px;
    overflow: hidden;
  }
  .screen .bar {
    position: absolute;
    right: 0;
    top: 85px;
    width: 6px;
    height: 80px;
    border-radius: 4px 0 0 4px;
    background: #2f6b66;
    animation: breathe 1.6s ease-in-out infinite;
  }
  .screen .panel {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 40%;
    background: #f6f3ee;
    border-left: 1px solid #ece6dc;
    padding: 18px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    transform: translateX(100%);
    transition: transform 0.45s cubic-bezier(0.16, 1, 0.3, 1);
  }
  .screen .cursor {
    position: absolute;
    top: 118px;
    right: 120px;
    width: 0;
    height: 0;
    border-left: 8px solid transparent;
    border-right: 8px solid transparent;
    border-bottom: 15px solid #1c1c1a;
    transform: rotate(-25deg);
    animation: approach 2.4s ease-in-out infinite;
  }
  .screen.done .panel {
    transform: none;
  }
  .screen.done .cursor,
  .screen.done .bar {
    animation: none;
    opacity: 0;
  }
  .screen.left .bar {
    right: auto;
    left: 0;
    border-radius: 0 4px 4px 0;
  }
  .screen.left .panel {
    right: auto;
    left: 0;
    border-left: none;
    border-right: 1px solid #ece6dc;
    transform: translateX(-100%);
  }
  .screen.left.done .panel {
    transform: none;
  }
  .screen.left .cursor {
    right: auto;
    left: 120px;
    animation-name: approach-left;
  }

  .keys {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 4vh;
  }
  kbd {
    min-width: 64px;
    padding: 18px 20px;
    border-radius: 14px;
    background: #fffdfa;
    box-shadow: 0 10px 30px rgba(120, 70, 30, 0.14), 0 0 0 1px rgba(60, 40, 20, 0.07), inset 0 -3px 0 #ece6dc;
    font: 600 20px/1 "Figtree Variable", system-ui, sans-serif;
    color: #1c1c1a;
    text-align: center;
    transition: transform 0.18s, box-shadow 0.18s;
  }
  .keys.done kbd {
    transform: translateY(2px);
    box-shadow: 0 6px 20px rgba(30, 120, 80, 0.18), 0 0 0 2px #3fae79, inset 0 -1px 0 #ece6dc;
  }
  .plus {
    color: #8c867c;
    font-size: 18px;
  }

  .devices {
    display: flex;
    align-items: flex-end;
    gap: 22px;
  }
  .laptop {
    width: 270px;
    height: 170px;
    padding: 22px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .phone {
    width: 92px;
    height: 176px;
    border-radius: 20px;
    background: #fffdfa;
    box-shadow: 0 18px 50px rgba(120, 70, 30, 0.18), 0 0 0 1px rgba(60, 40, 20, 0.08);
    padding: 24px 12px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 10px;
    position: relative;
    animation: float 4s ease-in-out infinite;
  }
  .notch {
    position: absolute;
    top: 9px;
    left: 50%;
    width: 26px;
    height: 5px;
    margin-left: -13px;
    border-radius: 3px;
    background: #ece7de;
  }

  .today {
    width: min(78%, 400px);
    padding: 18px 18px 10px;
    box-sizing: border-box;
  }
  .label {
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #9a948a;
    margin-bottom: 8px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 11px;
    font-size: 13px;
    padding: 11px 2px;
    border-top: 1px solid #f0ebe3;
    animation: rise 0.5s cubic-bezier(0.16, 1, 0.3, 1) both;
  }
  .row:nth-child(3) {
    animation-delay: 0.08s;
  }
  .row:nth-child(4) {
    animation-delay: 0.16s;
  }
  .dot {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    flex: none;
  }
  .dot.ig {
    background: #e1306c;
  }
  .dot.th,
  .dot.x {
    background: #1c1c1a;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.35; transform: scale(0.8); }
  }
  @keyframes breathe {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }
  @keyframes approach {
    0% { right: 140px; opacity: 0; }
    15% { opacity: 1; }
    65% { right: 10px; opacity: 1; }
    100% { right: 10px; opacity: 0; }
  }
  @keyframes approach-left {
    0% { left: 140px; opacity: 0; }
    15% { opacity: 1; }
    65% { left: 10px; opacity: 1; }
    100% { left: 10px; opacity: 0; }
  }
  @keyframes slidein {
    from { transform: translateX(100%); }
    to { transform: none; }
  }
  @keyframes float {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-6px); }
  }
  @keyframes rise {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: none; }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse,
    .screen .bar,
    .screen .cursor,
    .sheet,
    .phone,
    .row {
      animation: none;
    }
  }
  @media (max-width: 760px) {
    .split {
      grid-template-columns: 1fr;
    }
    .right {
      display: none;
    }
  }
</style>
