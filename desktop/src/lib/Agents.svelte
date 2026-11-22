<!-- AI agents: let Claude, Codex, Cursor, Kiro... use Sliptab through MCP. The server is this app,
     started by the agent as `<sliptab> mcp` (src-tauri/src/mcp.rs). Brand marks in static/logos
     (Simple Icons; Kiro's from kiro.dev), used only to name the agent. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { ago } from "./notes";

  type Access = "off" | "read" | "write" | "full";
  let { access, onaccess }: { access: Access; onaccess: (a: Access) => void } = $props();

  let command = $state("");
  let last: { client: string; tool: string; at: number } | null = $state(null);
  let subscribed = $state(true);
  async function load() {
    const info = await invoke<{ command: string; last: typeof last; subscribed: boolean }>("mcp_info");
    command = info.command;
    last = info.last;
    subscribed = info.subscribed;
  }
  $effect(() => {
    void load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  });

  const levels: { id: Access; name: string; what: string }[] = [
    { id: "off", name: "Off", what: "Agents can't see your notes." },
    { id: "read", name: "Read", what: "Agents can search and read your notes." },
    { id: "write", name: "Read & write", what: "Agents can also create, edit and pin notes." },
    { id: "full", name: "Full", what: "Agents can also delete notes, on every device." },
  ];
  const level = $derived(levels.find((l) => l.id === access) ?? levels[0]);

  const json = (key: string, extra: Record<string, string> = {}) =>
    JSON.stringify({ [key]: { sliptab: { ...extra, command, args: ["mcp"] } } }, null, 2);
  // logo: file in static/logos; tint: brand colour, or null for the text colour (black marks).
  const agents = $derived([
    { name: "Claude Code", logo: "claude", tint: "#D97757", where: "Run in a terminal", code: `claude mcp add sliptab -- "${command}" mcp` },
    { name: "Codex", logo: "openai", tint: null, where: "Run in a terminal", code: `codex mcp add sliptab -- "${command}" mcp` },
    { name: "Cursor", logo: "cursor", tint: null, where: "Add to ~/.cursor/mcp.json", code: json("mcpServers") },
    { name: "Kiro", logo: "kiro", tint: "img", where: "Add to ~/.kiro/settings/mcp.json", code: json("mcpServers") },
    { name: "Claude Desktop", logo: "claude", tint: "#D97757", where: "Settings → Developer → Edit config", code: json("mcpServers") },
    { name: "VS Code", logo: "visualstudiocode", tint: "#0A84D0", where: "Add to .vscode/mcp.json", code: json("servers", { type: "stdio" }) },
    { name: "Other", logo: "", tint: null, where: "Any MCP client, as a stdio server", code: `"${command}" mcp` },
  ]);
  let picked = $state("Claude Code");
  const shown = $derived(agents.find((a) => a.name === picked) ?? agents[0]);

  let copied = $state(false);
  async function copy(text: string) {
    try {
      await invoke("clip_write", { text });
    } catch {
      await navigator.clipboard.writeText(text);
    }
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

{#snippet mark(a: (typeof agents)[number], size: number)}
  {#if a.tint === "img"}
    <img src="/logos/{a.logo}.svg" alt="" width={size} height={size} />
  {:else if a.logo}
    <i class="mark" style:--src="url(/logos/{a.logo}.svg)" style:--tint={a.tint ?? "var(--text)"} style:width="{size}px" style:height="{size}px"></i>
  {:else}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m8 8-4 4 4 4M16 8l4 4-4 4"/></svg>
  {/if}
{/snippet}

<div class="agents">
  <header>
    <h1>AI agents</h1>
    <p>Let your AI agent read and update your notes. Stays on this computer.</p>
  </header>

  {#if !subscribed}
    <div class="plan">AI agents come with sync. Sign in under Settings → Sync to start your 30 free days.</div>
  {/if}

  <section>
    <div class="label">Access</div>
    <div class="seg" role="radiogroup" aria-label="Agent access">
      {#each levels as l (l.id)}
        <button role="radio" aria-checked={access === l.id} class:on={access === l.id} class:full={l.id === "full"} onclick={() => onaccess(l.id)}>{l.name}</button>
      {/each}
    </div>
    <p class="what" class:danger={access === "full"}>{level.what}</p>
  </section>

  <section>
    <div class="label">Connect</div>
    <div class="grid">
      {#each agents as a (a.name)}
        <button class="agent" class:on={picked === a.name} onclick={() => (picked = a.name)}>
          {@render mark(a, 22)}
          <span>{a.name}</span>
        </button>
      {/each}
    </div>
    <div class="code">
      <div class="where">{shown.where}, then restart {shown.name === "Other" ? "the agent" : shown.name}.</div>
      <pre>{shown.code}</pre>
      <button class="copy" onclick={() => copy(shown.code)}>{copied ? "Copied" : "Copy"}</button>
    </div>
  </section>

  <footer class:live={!!last}>
    <span class="dot"></span>
    {#if last}{last.client} · {ago(last.at)}{:else}No agent connected yet{/if}
  </footer>
</div>

<style>
  .agents {
    overflow: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 30px;
    max-width: 640px;
  }
  h1 {
    font-size: 24px;
    font-weight: 600;
    letter-spacing: -0.01em;
    margin: 0;
    color: var(--text);
  }
  header p {
    margin: 6px 0 0;
    color: var(--muted);
    font-size: 13.5px;
  }
  .plan {
    padding: 11px 14px;
    border-radius: 12px;
    font-size: 13px;
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--faint);
    margin-bottom: 10px;
  }
  .seg {
    display: inline-flex;
    padding: 3px;
    gap: 2px;
    border-radius: 11px;
    background: var(--surface-2);
  }
  .seg button {
    all: unset;
    cursor: pointer;
    padding: 7px 15px;
    border-radius: 8px;
    font-size: 13px;
    color: var(--muted);
    transition: background 0.15s, color 0.15s;
  }
  .seg button:hover {
    color: var(--text);
  }
  .seg button.on {
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
    box-shadow: 0 1px 3px var(--shadow);
  }
  .seg button.on.full {
    color: #e5484d;
  }
  .what {
    margin: 10px 0 0;
    font-size: 12.5px;
    color: var(--muted);
  }
  .what.danger {
    color: #e5484d;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(96px, 1fr));
    gap: 8px;
  }
  .agent {
    all: unset;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 14px 6px 11px;
    border-radius: 12px;
    border: 1px solid var(--line);
    color: var(--text);
    transition: border-color 0.15s, background 0.15s;
  }
  .agent span {
    font-size: 11.5px;
    color: var(--muted);
    text-align: center;
  }
  .agent:hover {
    background: var(--surface-2);
  }
  .agent.on {
    border-color: var(--text);
  }
  .agent.on span {
    color: var(--text);
  }
  .agent img {
    border-radius: 5px;
  }
  .mark {
    display: block;
    background: var(--tint);
    -webkit-mask: var(--src) center / contain no-repeat;
    mask: var(--src) center / contain no-repeat;
  }
  .code {
    position: relative;
    margin-top: 12px;
    border-radius: 12px;
    background: var(--surface-2);
    padding: 12px 14px;
  }
  .where {
    font-size: 12px;
    color: var(--muted);
    margin-bottom: 8px;
    padding-right: 64px;
  }
  .code pre {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  .copy {
    all: unset;
    cursor: pointer;
    position: absolute;
    top: 9px;
    right: 9px;
    padding: 4px 11px;
    border-radius: 7px;
    font-size: 12px;
    color: var(--text);
    background: var(--surface);
    box-shadow: 0 1px 2px var(--shadow);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--faint);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--faint);
  }
  footer.live {
    color: var(--muted);
  }
  footer.live .dot {
    background: #30a46c;
    box-shadow: 0 0 0 3px color-mix(in srgb, #30a46c 25%, transparent);
  }
</style>
