<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    fetchWorlds,
    switchWorld,
    createWorld,
    fetchWorldGenStatus,
    type WorldEntry,
    type WorldGenStatus,
  } from "./api";

  let { session }: { session: string } = $props();

  let worlds = $state<WorldEntry[]>([]);
  let error = $state("");
  let loading = $state(true);
  let confirmTarget = $state<WorldEntry | null>(null);
  let switching = $state(false);
  let switched = $state(false);

  // ---- world creation ----
  const SIZES = [
    { label: "tiny (fast, for testing)", width: 1600, height: 600 },
    { label: "small — 4200 × 1200", width: 4200, height: 1200 },
    { label: "medium — 6400 × 1800", width: 6400, height: 1800 },
    { label: "large — 8400 × 2400", width: 8400, height: 2400 },
  ];
  let newName = $state("");
  let sizeIndex = $state(1);
  let seed = $state("");
  let gen = $state<WorldGenStatus | null>(null);
  let genError = $state("");
  let pollId: ReturnType<typeof setInterval> | null = null;

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

  onMount(async () => {
    await refresh();
    // If a generation is already running (e.g. after navigating away and back), pick it up.
    try {
      const s = await fetchWorldGenStatus(session);
      if (s.status !== "idle") {
        gen = s;
        if (s.running) startPolling();
      }
    } catch {
      // ignore — the status endpoint is best-effort here
    }
  });

  onDestroy(() => {
    if (pollId) clearInterval(pollId);
  });

  function startPolling() {
    if (pollId) clearInterval(pollId);
    pollId = setInterval(async () => {
      try {
        gen = await fetchWorldGenStatus(session);
        if (!gen.running) {
          if (pollId) clearInterval(pollId);
          pollId = null;
          await refresh();
        }
      } catch (e) {
        genError = e instanceof Error ? e.message : String(e);
      }
    }, 1000);
  }

  async function generate(e: SubmitEvent) {
    e.preventDefault();
    genError = "";
    const size = SIZES[sizeIndex];
    try {
      gen = await createWorld(session, {
        name: newName.trim(),
        width: size.width,
        height: size.height,
        seed: seed.trim() || undefined,
      });
      newName = "";
      seed = "";
      startPolling();
    } catch (err) {
      genError = err instanceof Error ? err.message : String(err);
    }
  }

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
{:else}
  {#if loading}
    <p class="dim">loading…</p>
  {:else if worlds.length === 0}
    <p class="dim">no worlds found in the Terraria world directory on this machine yet.</p>
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

  <section class="create">
    <h3 class="dim">create a new world</h3>
    <p class="dim">
      generation runs in the background on the server — it can take from a second (tiny) to a good
      while (large). the new world lands in the Terraria world directory; switch to it here once it
      is done.
    </p>

    {#if genError}<p class="danger">{genError}</p>{/if}

    {#if gen && gen.status !== "idle"}
      <div class="job" class:done={gen.status === "done"} class:failed={gen.status === "failed"}>
        <div class="job-head">
          <span class="job-name">{gen.name || "world"}</span>
          <span class="job-status">{gen.status}</span>
        </div>
        {#if gen.running}
          <div class="bar"><div class="bar-fill"></div></div>
          <span class="dim">generating… {gen.elapsed_secs}s elapsed</span>
        {:else if gen.status === "done"}
          <span class="accent-text">{gen.message}</span>
        {:else}
          <span class="danger">{gen.message}</span>
        {/if}
      </div>
    {/if}

    <form onsubmit={generate}>
      <label>
        name
        <input bind:value={newName} placeholder="My New World" autocomplete="off" required />
      </label>
      <label>
        size
        <select bind:value={sizeIndex}>
          {#each SIZES as s, i (s.label)}
            <option value={i}>{s.label}</option>
          {/each}
        </select>
      </label>
      <label>
        seed <span class="hint">optional</span>
        <input bind:value={seed} placeholder="blank = random" autocomplete="off" />
      </label>
      <button type="submit" disabled={!newName.trim() || (gen?.running ?? false)}>
        {gen?.running ? "generating…" : "generate"}
      </button>
    </form>
  </section>
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

  h3 {
    margin: 0 0 0.5rem;
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-weight: 500;
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

  .create {
    margin-top: 2rem;
    max-width: 640px;
  }

  .create > p {
    max-width: 46em;
  }

  form {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 0.75rem;
    margin-top: 1rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .hint {
    color: var(--text-dim);
    font-size: 0.68rem;
  }

  select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 2px;
    padding: 0.5rem 0.4rem;
    font-family: inherit;
  }

  .job {
    border: 1px solid var(--border);
    border-left: 3px solid var(--warn);
    background: var(--bg-raised);
    border-radius: 4px;
    padding: 0.7rem 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    margin: 1rem 0;
  }

  .job.done {
    border-left-color: var(--accent);
  }

  .job.failed {
    border-left-color: var(--danger);
  }

  .job-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .job-name {
    color: var(--text);
    font-weight: 600;
  }

  .job-status {
    text-transform: uppercase;
    font-size: 0.7rem;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }

  .bar {
    height: 6px;
    background: var(--bg);
    border-radius: 3px;
    overflow: hidden;
  }

  /* An indeterminate sweep — worldgen has no real percentage to report. */
  .bar-fill {
    height: 100%;
    width: 35%;
    background: var(--warn);
    border-radius: 3px;
    animation: sweep 1.2s ease-in-out infinite;
  }

  @keyframes sweep {
    0% {
      margin-left: -35%;
    }
    100% {
      margin-left: 100%;
    }
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
    text-transform: none;
    letter-spacing: 0;
    font-size: 1rem;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }
</style>
