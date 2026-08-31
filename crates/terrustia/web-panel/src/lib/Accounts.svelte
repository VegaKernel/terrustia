<script lang="ts">
  import { onMount } from "svelte";
  import {
    fetchAccounts,
    setAccountGroup,
    createAccount,
    deleteAccount,
    fetchKnownPermissions,
    setGroupPermission,
    ApiCallError,
    type AccountsState,
    type AccountInfo,
  } from "./api";

  let { session }: { session: string } = $props();

  let view = $state<AccountsState | null>(null);
  let error = $state("");
  let busy = $state<string | null>(null);

  let newName = $state("");
  let newPassword = $state("");
  let newGroup = $state("");
  let creating = $state(false);
  let createNote = $state("");

  let deleteTarget = $state<AccountInfo | null>(null);

  // The group-permission editor. `known` is `null` until it loads and stays `null` (rather than an
  // empty list) if the session lacks `admin.groups` — a `403` here means "you may see the accounts
  // list but not edit what a group can do", so the editor controls simply do not appear, matching
  // exactly what the backend would refuse anyway.
  let known = $state<string[] | null>(null);
  let editingGroup = $state<string | null>(null);
  let permBusy = $state(false);
  let permError = $state("");

  async function refresh() {
    try {
      view = await fetchAccounts(session);
      if (!newGroup && view.groups.length) newGroup = view.groups[0].name;
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    try {
      known = await fetchKnownPermissions(session);
    } catch (e) {
      // A 403 (no `admin.groups`) is expected and quiet; anything else is worth knowing about.
      known = null;
      if (!(e instanceof ApiCallError)) throw e;
    }
  }

  onMount(refresh);

  async function togglePermission(group: string, permission: string, grant: boolean) {
    permBusy = true;
    permError = "";
    try {
      await setGroupPermission(session, group, permission, grant);
      await refresh();
    } catch (e) {
      permError = e instanceof Error ? e.message : String(e);
    } finally {
      permBusy = false;
    }
  }

  async function changeGroup(a: AccountInfo, group: string) {
    if (group === a.group) return;
    busy = a.name;
    error = "";
    try {
      await setAccountGroup(session, a.name, group);
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      await refresh(); // put the select back to the real value
    } finally {
      busy = null;
    }
  }

  async function create(e: SubmitEvent) {
    e.preventDefault();
    creating = true;
    createNote = "";
    error = "";
    try {
      await createAccount(session, newName.trim(), newPassword, newGroup);
      createNote = `created ${newName.trim()} in ${newGroup}.`;
      newName = "";
      newPassword = "";
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      creating = false;
    }
  }

  async function confirmDelete() {
    if (!deleteTarget) return;
    busy = deleteTarget.name;
    error = "";
    try {
      await deleteAccount(session, deleteTarget.name);
      deleteTarget = null;
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = null;
    }
  }
</script>

<h2>accounts &amp; permissions</h2>

{#if error}<p class="danger">{error}</p>{/if}

{#if view}
  <section>
    <h3 class="dim">groups</h3>
    {#if permError}<p class="danger">{permError}</p>{/if}
    <div class="groups">
      {#each view.groups as g (g.name)}
        <div class="group" class:admin={g.can_admin}>
          <span class="group-name">{g.name}</span>
          {#if g.can_admin}<span class="badge">admin</span>{/if}
          {#if known}
            <button
              class="edit-link"
              onclick={() => (editingGroup = editingGroup === g.name ? null : g.name)}
            >
              {editingGroup === g.name ? "done" : "edit"}
            </button>
          {/if}
          <div class="perms">
            {#each g.permissions as p (p)}
              <span class="perm" class:star={p === "*"}>{p === "*" ? "all (*)" : p}</span>
            {/each}
            {#if g.permissions.length === 0}<span class="dim">(none)</span>{/if}
          </div>
          {#if known && editingGroup === g.name}
            <div class="perm-editor">
              {#each known as p (p)}
                <label class="perm-toggle">
                  <input
                    type="checkbox"
                    checked={g.permissions.includes(p)}
                    disabled={permBusy}
                    onchange={(e) =>
                      togglePermission(g.name, p, (e.currentTarget as HTMLInputElement).checked)}
                  />
                  <span class:star={p === "*"}>{p === "*" ? "all (*)" : p}</span>
                </label>
              {/each}
              <p class="hint">
                you can only grant a permission you hold yourself; the backend refuses anything else.
              </p>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  </section>

  <section>
    <h3 class="dim">accounts <span class="dim">({view.accounts.length})</span></h3>
    {#if view.accounts.length === 0}
      <p class="dim">no accounts yet.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>name</th>
            <th>group</th>
            <th>admin</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each view.accounts as a (a.name)}
            <tr>
              <td>{a.name}</td>
              <td>
                <select
                  value={a.group}
                  disabled={busy === a.name}
                  onchange={(e) => changeGroup(a, (e.currentTarget as HTMLSelectElement).value)}
                >
                  {#each view.groups as g (g.name)}
                    <option value={g.name}>{g.name}</option>
                  {/each}
                  {#if !view.groups.some((g) => g.name === a.group)}
                    <option value={a.group}>{a.group} (unknown)</option>
                  {/if}
                </select>
              </td>
              <td>{a.can_admin ? "yes" : "—"}</td>
              <td class="actions">
                <button
                  class="danger-btn"
                  disabled={busy === a.name}
                  onclick={() => (deleteTarget = a)}>delete</button
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section>
    <h3 class="dim">create an account</h3>
    <form onsubmit={create}>
      <label>
        name
        <input bind:value={newName} autocomplete="off" required />
      </label>
      <label>
        password
        <input bind:value={newPassword} type="password" autocomplete="new-password" required />
        <span class="hint">at least six characters</span>
      </label>
      <label>
        group
        <select bind:value={newGroup}>
          {#each view.groups as g (g.name)}
            <option value={g.name}>{g.name}</option>
          {/each}
        </select>
      </label>
      <button type="submit" disabled={creating || !newName.trim() || newPassword.length < 6}>
        {creating ? "creating…" : "create account"}
      </button>
    </form>
    {#if createNote}<p class="accent-text">{createNote}</p>{/if}
  </section>
{:else}
  <p class="dim">loading…</p>
{/if}

{#if deleteTarget}
  <div
    class="overlay"
    role="presentation"
    onkeydown={(e) => e.key === "Escape" && (deleteTarget = null)}
  >
    <div class="dialog" role="dialog" aria-label="confirm delete" tabindex="-1">
      <h3 class="danger">delete {deleteTarget.name}?</h3>
      <!-- What actually happens: `PanelAuthorize` resolves through `account_hash_and_group`, which
           returns nothing for a deleted account, so every panel request under that session is
           refused from the next one onwards. In-game, `Admin::group_of` falls back to `default` for
           an unknown account, so those privileges go too. The one thing that survives is a
           WebSocket that was already open when the account went, because both sockets are
           authorized at upgrade and never re-checked. -->
      <p class="dim">
        this removes the account permanently. every panel request under it is refused from the next
        one onwards, and in game it drops to the default group. a live feed already open in a
        browser keeps streaming until that socket drops.
      </p>
      <div class="dialog-actions">
        <button onclick={() => (deleteTarget = null)}>cancel</button>
        <button class="danger-btn" onclick={confirmDelete}>confirm delete</button>
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

  section {
    margin-bottom: 1.75rem;
  }

  .accent-text {
    color: var(--accent);
  }

  .groups {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 0.6rem;
    max-width: 720px;
  }

  .group {
    border: 1px solid var(--border);
    background: var(--bg-raised);
    border-radius: 4px;
    padding: 0.7rem 0.8rem;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
  }

  .group.admin {
    border-color: var(--accent-dim);
  }

  .group-name {
    color: var(--accent);
    font-weight: 600;
  }

  .badge {
    font-size: 0.62rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--bg);
    background: var(--accent);
    border-radius: 2px;
    padding: 0.05rem 0.35rem;
  }

  .perms {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    width: 100%;
  }

  .perm {
    font-size: 0.72rem;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 2px;
    padding: 0.05rem 0.4rem;
  }

  .perm.star {
    color: var(--accent);
    border-color: var(--accent-dim);
  }

  .edit-link {
    margin-left: auto;
    background: transparent;
    border: none;
    color: var(--accent);
    font-size: 0.72rem;
    padding: 0;
    cursor: pointer;
  }

  .edit-link:hover {
    text-decoration: underline;
  }

  .perm-editor {
    width: 100%;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 0.9rem;
    margin-top: 0.5rem;
    padding-top: 0.5rem;
    border-top: 1px dashed var(--border);
  }

  .perm-toggle {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.75rem;
    color: var(--text-dim);
  }

  .perm-toggle .star {
    color: var(--accent);
  }

  .perm-editor .hint {
    width: 100%;
    margin: 0.3rem 0 0;
  }

  table {
    width: 100%;
    max-width: 620px;
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

  select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 2px;
    padding: 0.35rem 0.4rem;
    font-family: inherit;
    font-size: 0.85rem;
  }

  .actions {
    text-align: right;
  }

  form {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 0.75rem;
    max-width: 620px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .hint {
    font-size: 0.68rem;
    color: var(--text-dim);
  }

  .danger-btn {
    background: transparent;
    border-color: var(--danger);
    color: var(--danger);
    padding: 0.3rem 0.6rem;
    font-size: 0.8rem;
  }

  .danger-btn:hover:not(:disabled) {
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
