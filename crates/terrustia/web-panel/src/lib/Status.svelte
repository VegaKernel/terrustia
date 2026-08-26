<script lang="ts">
  import { onDestroy } from "svelte";
  import { watchStatus, logout, type StatusResponse } from "./api";

  let { session, onLoggedOut }: { session: string; onLoggedOut: () => void } = $props();

  let status = $state<StatusResponse | null>(null);
  let live = $state(false);

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
      live = true;
    },
    () => {
      live = false;
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
  <span class="spacer"></span>
  <button onclick={signOut}>sign out</button>
</header>

<main>
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
    <p class="dim note">
      This is the panel's foundation — a live status view over a real WebSocket connection to the
      running server. Player list, world controls, and the rest land in follow-up work.
    </p>
  {:else}
    <p class="dim">waiting for the server…</p>
  {/if}
</main>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--bg-raised);
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

  .spacer {
    flex: 1;
  }

  main {
    flex: 1;
    padding: 1.5rem;
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

  .note {
    margin-top: 1.5rem;
    max-width: 46em;
  }
</style>
