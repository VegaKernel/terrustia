<script lang="ts">
  import { onMount } from "svelte";
  import {
    fetchBackups,
    forceSave,
    rollback,
    type Backups,
    type BackupEntry,
  } from "./api";

  let { session }: { session: string } = $props();

  let view = $state<Backups | null>(null);
  let error = $state("");
  let saving = $state(false);
  let saveNote = $state("");
  let confirmTarget = $state<BackupEntry | null>(null);
  let rollingBack = $state(false);
  let rolledBack = $state("");

  async function refresh() {
    try {
      view = await fetchBackups(session);
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(() => {
    refresh();
    const id = setInterval(refresh, 4000);
    return () => clearInterval(id);
  });

  async function doSave() {
    saving = true;
    saveNote = "";
    try {
      await forceSave(session);
      saveNote = "save requested  -  the list below refreshes automatically.";
      // Give the background save a beat to rotate the backups, then refresh.
      setTimeout(refresh, 1500);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  async function doRollback() {
    if (!confirmTarget) return;
    rollingBack = true;
    try {
      const message = await rollback(session, confirmTarget.index);
      rolledBack = message;
      confirmTarget = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      rollingBack = false;
    }
  }

  function age(secs: number | null): string {
    if (secs == null) return "unknown";
    if (secs < 90) return `${secs}s ago`;
    if (secs < 5400) return `${Math.round(secs / 60)}m ago`;
    return `${(secs / 3600).toFixed(1)}h ago`;
  }
</script>

<h2>backups &amp; rollback</h2>

{#if error}<p class="danger">{error}</p>{/if}

{#if rolledBack}
  <p class="warn">
    {rolledBack}
  </p>
  <p class="dim">
    the server process is stopping so the restored world loads cleanly on the next start. this panel
    will go offline until the server is started again.
  </p>
{:else if view}
  {#if !view.saving}
    <p class="dim">
      this world is not being saved (no save destination), so there is nothing to back up or roll
      back. start the server with a save file to enable backups.
    </p>
  {:else}
    <p class="dim">
      backups of <span class="accent-text">{view.world_file ?? "the world"}</span>. one is written
      each time the world is saved; the {view.kept} most recent are kept, rotating.
      <span class="mono">bak1</span> is the newest.
    </p>

    <div class="toolbar">
      <button onclick={doSave} disabled={saving}>{saving ? "saving…" : "save now"}</button>
      {#if saveNote}<span class="dim note">{saveNote}</span>{/if}
    </div>

    {#if view.backups.length === 0}
      <p class="dim">no backups yet — press “save now”, and one will appear.</p>
    {:else}
      <div class="table-scroll">
        <table>
          <thead>
            <tr>
              <th>#</th>
              <th>size</th>
              <th>age</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each view.backups as b (b.index)}
              <tr>
                <td class="mono">bak{b.index}{#if b.index === 1}<span class="dim"> · newest</span>{/if}</td>
                <td class="dim">{b.size_mb.toFixed(2)} MB</td>
                <td class="dim">{age(b.age_secs)}</td>
                <td class="actions">
                  <button class="danger-btn" onclick={() => (confirmTarget = b)}>roll back to this</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/if}
{:else}
  <p class="dim">loading…</p>
{/if}

{#if confirmTarget}
  <div
    class="overlay"
    role="presentation"
    onkeydown={(e) => e.key === "Escape" && !rollingBack && (confirmTarget = null)}
  >
    <div class="dialog" role="dialog" aria-label="confirm rollback" tabindex="-1">
      <h3 class="danger">roll back to bak{confirmTarget.index}?</h3>
      <p class="dim">
        this is destructive. the current world is set aside (as
        <span class="mono">.wld.before-rollback</span>) and replaced with backup #{confirmTarget.index},
        then <strong>the server stops</strong> so the restored world loads cleanly on the next start.
        every connected player is disconnected. this cannot be undone from the panel once the server
        is down.
      </p>
      <div class="dialog-actions">
        <button onclick={() => (confirmTarget = null)} disabled={rollingBack}>cancel</button>
        <button class="danger-btn" onclick={doRollback} disabled={rollingBack}>
          {rollingBack ? "rolling back…" : "confirm rollback"}
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

  .mono {
    font-family: var(--mono);
    color: var(--text);
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin: 1rem 0;
  }

  .note {
    font-size: 0.8rem;
  }

  table {
    width: 100%;
    max-width: 560px;
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
    font-size: 0.72rem;
    letter-spacing: 0.06em;
  }

  .actions {
    text-align: right;
  }

  .danger-btn {
    background: transparent;
    border-color: var(--danger);
    color: var(--danger);
    padding: 0.3rem 0.6rem;
    font-size: 0.8rem;
  }

  .danger-btn:hover {
    background: var(--danger);
    color: var(--bg);
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
    border-radius: var(--radius-lg);
    padding: 1.25rem;
    width: min(420px, 92vw);
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
