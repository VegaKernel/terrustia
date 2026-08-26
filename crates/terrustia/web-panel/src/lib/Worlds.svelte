<script lang="ts">
  import { onMount } from "svelte";
  import { fetchWorlds, switchWorld, type WorldEntry } from "./api";

  let { session }: { session: string } = $props();

  let worlds = $state<WorldEntry[]>([]);
  let error = $state("");
  let loading = $state(true);
  let confirmTarget = $state<WorldEntry | null>(null);
  let switching = $state(false);
  let switched = $state(false);

  async function refresh() {
    try {
      worlds = await fetchWorlds(session);
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(refresh);

  async function doSwitch() {
    if (!confirmTarget) return;
    switching = true;
    try {
      await switchWorld(session, confirmTarget.name);
      switched = true;
      confirmTarget = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      switching = false;
    }
  }
</script>

<h2>worlds</h2>

{#if error}<p class="danger">{error}</p>{/if}

{#if switched}
  <p class="warn">
    the server is restarting into the new world. this page will reconnect once it is back up — a
    real graceful restart, not a hot-swap, so it may take a few seconds.
  </p>
{:else if loading}
  <p class="dim">loading…</p>
{:else if worlds.length === 0}
  <p class="dim">no worlds found in the Terraria world directory on this machine.</p>
{:else}
  <table>
    <thead>
      <tr>
        <th>name</th>
        <th>size</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each worlds as w (w.name)}
        <tr>
          <td>
            {w.name}
            {#if w.current}<span class="accent-text"> · running</span>{/if}
          </td>
          <td class="dim">{w.size_mb.toFixed(1)} MB</td>
          <td class="actions">
            {#if !w.current}
              <button onclick={() => (confirmTarget = w)}>switch to this world</button>
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

{#if confirmTarget}
  <div
    class="overlay"
    role="presentation"
    onkeydown={(e) => e.key === "Escape" && (confirmTarget = null)}
  >
    <div class="dialog" role="dialog" aria-label="switch world" tabindex="-1">
      <h3>switch to {confirmTarget.name}?</h3>
      <p class="dim">
        this restarts the server process: the current world is saved, every connected player is
        disconnected, and the process comes back up serving {confirmTarget.name} instead. it is a
        real process restart, not a live hot-swap — that is the only honest way to do this safely.
      </p>
      <div class="dialog-actions">
        <button onclick={() => (confirmTarget = null)} disabled={switching}>cancel</button>
        <button onclick={doSwitch} disabled={switching}>
          {switching ? "switching…" : "confirm switch"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  h2 {
    margin: 0 0 1rem;
    font-size: 1rem;
    font-weight: 600;
  }

  .accent-text {
    color: var(--accent);
  }

  table {
    width: 100%;
    max-width: 640px;
    border-collapse: collapse;
    font-size: 0.88rem;
  }

  th,
  td {
    text-align: left;
    padding: 0.5rem 0.6rem;
    border-bottom: 1px solid var(--border);
  }

  th {
    color: var(--text-dim);
    font-weight: 500;
    text-transform: uppercase;
    font-size: 0.72rem;
    letter-spacing: 0.06em;
  }

  .actions {
    text-align: right;
  }

  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10;
  }

  .dialog {
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1.25rem;
    width: 380px;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .dialog h3 {
    margin: 0;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }
</style>
