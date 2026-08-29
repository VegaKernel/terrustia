<script lang="ts">
  import { onDestroy } from "svelte";
  import {
    watchStatus,
    logout,
    hasPermission,
    type StatusResponse,
    type ConsoleFeedLine,
  } from "./api";
  import Players from "./Players.svelte";
  import Whitelist from "./Whitelist.svelte";
  import Worlds from "./Worlds.svelte";
  import Console from "./Console.svelte";
  import Settings from "./Settings.svelte";
  import WorldView from "./WorldView.svelte";
  import Metrics from "./Metrics.svelte";
  import Backups from "./Backups.svelte";
  import Accounts from "./Accounts.svelte";
  import Audit from "./Audit.svelte";

  let { session, onLoggedOut }: { session: string; onLoggedOut: () => void } = $props();

  type Tab =
    | "overview"
    | "metrics"
    | "players"
    | "whitelist"
    | "accounts"
    | "worlds"
    | "backups"
    | "console"
    | "world"
    | "settings"
    | "audit";
  // Every tab except "overview" needs a permission to be worth showing — this is a UX convenience
  // only, gating which buttons a session sees; every route the tab talks to re-checks its own
  // permission on the backend regardless (see `panel/mod.rs`'s module doc for the full map), so a
  // stale or wrong entry here is a display bug, never a way past the real check.
  const ALL_TABS: { id: Tab; label: string; needs: string | null }[] = [
    { id: "overview", label: "overview", needs: null },
    { id: "metrics", label: "metrics", needs: "panel.view" },
    { id: "players", label: "players", needs: "panel.view" },
    { id: "whitelist", label: "whitelist", needs: "panel.view" },
    { id: "accounts", label: "accounts", needs: "admin.accounts" },
    { id: "worlds", label: "worlds", needs: "panel.view" },
    { id: "backups", label: "backups", needs: "panel.view" },
    { id: "console", label: "console", needs: "panel.console" },
    { id: "world", label: "world", needs: "panel.view" },
    { id: "settings", label: "settings", needs: "panel.view" },
    { id: "audit", label: "audit", needs: "admin.audit" },
  ];

  let tab = $state<Tab>("overview");
  let status = $state<StatusResponse | null>(null);
  let live = $state(false);

  let TABS = $derived(
    ALL_TABS.filter((t) => t.needs === null || hasPermission(status?.permissions ?? [], t.needs)),
  );

  // If the tab currently open stops being one this session may see (permissions loaded in after
  // the initial render, or were changed mid-session), fall back to the one tab everyone always has.
  $effect(() => {
    if (!TABS.some((t) => t.id === tab)) tab = "overview";
  });
  const MAX_LINES = 400;
  let consoleLines = $state<ConsoleFeedLine[]>([]);

  // `session` is read once, here, on purpose: `watchStatus` takes a plain string and opens one
  // socket for this component's whole lifetime (closed in `onDestroy` below) — it was never meant
  // to react to a later change, and this app never re-issues a session into an already-mounted
  // `Status` anyway (a new login remounts the whole view). The compiler can't know that from the
  // call site alone.
  // svelte-ignore state_referenced_locally
  const stop = watchStatus(
    session,
    (s) => {
      status = s;
    },
    (line) => {
      consoleLines.push(line);
      if (consoleLines.length > MAX_LINES) consoleLines.splice(0, consoleLines.length - MAX_LINES);
    },
    (isLive) => {
      live = isLive;
    },
  );
  onDestroy(stop);

  function signOut() {
    logout();
    onLoggedOut();
  }

  function uptime(secs: number): string {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = Math.floor(secs % 60);
    return `${h}h ${m}m ${s}s`;
  }
</script>

<header>
  <span class="dot" class:live></span>
  <strong>terrustia</strong>
  {#if status}<span class="dim">v{status.version}</span>{/if}
  <nav>
    {#each TABS as t (t.id)}
      <button class="tab" class:active={tab === t.id} onclick={() => (tab = t.id)}>{t.label}</button>
    {/each}
  </nav>
  <span class="spacer"></span>
  <button onclick={signOut}>sign out</button>
</header>

<main class:no-pad={tab === "world"}>
  {#if tab === "overview"}
    {#if status}
      <div class="grid">
        <div class="card">
          <span class="label">world</span>
          <span class="value">{status.world_name}</span>
        </div>
        <div class="card">
          <span class="label">players</span>
          <span class="value">{status.player_count} / {status.max_players}</span>
        </div>
        <div class="card">
          <span class="label">uptime</span>
          <span class="value">{uptime(status.uptime_secs)}</span>
        </div>
      </div>
    {:else}
      <p class="dim">waiting for the server…</p>
    {/if}
  {:else if tab === "metrics"}
    <Metrics {session} />
  {:else if tab === "players"}
    <Players {session} />
  {:else if tab === "whitelist"}
    <Whitelist {session} />
  {:else if tab === "accounts"}
    <Accounts {session} />
  {:else if tab === "worlds"}
    <Worlds {session} />
  {:else if tab === "backups"}
    <Backups {session} />
  {:else if tab === "console"}
    <Console {session} lines={consoleLines} {live} />
  {:else if tab === "world"}
    <WorldView {session} />
  {:else if tab === "settings"}
    <Settings {session} />
  {:else if tab === "audit"}
    <Audit {session} />
  {/if}
</main>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--bg-raised);
    flex-wrap: wrap;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--danger);
    flex-shrink: 0;
  }

  .dot.live {
    background: var(--accent);
  }

  nav {
    display: flex;
    gap: 0.2rem;
    margin-left: 1rem;
  }

  .tab {
    background: transparent;
    border: 1px solid transparent;
    color: var(--text-dim);
    padding: 0.35rem 0.7rem;
    font-size: 0.85rem;
  }

  .tab:hover {
    background: var(--bg);
    color: var(--text);
  }

  .tab.active {
    color: var(--accent);
    border-color: var(--accent-dim);
    background: var(--accent-dim);
  }

  .spacer {
    flex: 1;
  }

  main {
    flex: 1;
    padding: 1.5rem;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  main.no-pad {
    padding: 0;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 0.75rem;
    max-width: 640px;
  }

  .card {
    border: 1px solid var(--border);
    background: var(--bg-raised);
    border-radius: 4px;
    padding: 0.9rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .label {
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-dim);
  }

  .value {
    font-size: 1.1rem;
    color: var(--accent);
  }
</style>
