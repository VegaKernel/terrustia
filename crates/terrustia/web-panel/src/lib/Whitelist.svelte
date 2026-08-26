<script lang="ts">
  import { onMount } from "svelte";
  import { fetchWhitelist, addToWhitelist, removeFromWhitelist, type WhitelistState } from "./api";

  let { session }: { session: string } = $props();

  let list = $state<WhitelistState | null>(null);
  let error = $state("");
  let newName = $state("");
  let busy = $state(false);

  async function refresh() {
    try {
      list = await fetchWhitelist(session);
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(refresh);

  async function add() {
    const name = newName.trim();
    if (!name) return;
    busy = true;
    try {
      await addToWhitelist(session, name);
      newName = "";
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function remove(name: string) {
    busy = true;
    try {
      await removeFromWhitelist(session, name);
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<h2>whitelist</h2>

{#if error}<p class="danger">{error}</p>{/if}

{#if list}
  <p class="dim">
    {#if list.on}
      the guest list is <span class="accent-text">on</span> — only these {list.names.length} name(s)
      may join.
    {:else}
      the guest list is <span class="dim">off</span> — anyone may join. adding a name turns it on.
    {/if}
  </p>

  <form onsubmit={(e) => { e.preventDefault(); add(); }}>
    <input placeholder="player name" bind:value={newName} disabled={busy} />
    <button disabled={busy || !newName.trim()}>add</button>
  </form>

  {#if list.names.length > 0}
    <ul>
      {#each list.names as name (name)}
        <li>
          <span>{name}</span>
          <button class="danger-btn" disabled={busy} onclick={() => remove(name)}>remove</button>
        </li>
      {/each}
    </ul>
  {/if}
{:else}
  <p class="dim">loading…</p>
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

  form {
    display: flex;
    gap: 0.5rem;
    margin: 1rem 0;
    max-width: 420px;
  }

  form input {
    flex: 1;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-width: 420px;
  }

  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 0.7rem;
    border-bottom: 1px solid var(--border);
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
</style>
