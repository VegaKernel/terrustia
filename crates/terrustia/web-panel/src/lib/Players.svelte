<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { fetchPlayers, kickPlayer, banPlayer, type Player, type BanKind } from "./api";

  let { session }: { session: string } = $props();

  let players = $state<Player[]>([]);
  let error = $state("");
  let acting = $state<string | null>(null);
  let banTarget = $state<Player | null>(null);
  let banKind = $state<BanKind>("name");
  let banValue = $state("");
  let banReason = $state("");

  async function refresh() {
    try {
      players = await fetchPlayers(session);
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(() => {
    refresh();
    const id = setInterval(refresh, 2000);
    return () => clearInterval(id);
  });

  async function kick(p: Player) {
    acting = p.name;
    try {
      await kickPlayer(session, p.name, "kicked from the web panel");
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      acting = null;
    }
  }

  function openBan(p: Player) {
    banTarget = p;
    banKind = "name";
    banValue = p.name;
    banReason = "";
  }

  async function confirmBan() {
    if (!banTarget) return;
    acting = banTarget.name;
    try {
      await banPlayer(session, banKind, banValue, banReason);
      banTarget = null;
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      acting = null;
    }
  }

  function health(p: Player): string {
    return `${p.life} / ${p.life_max}`;
  }

  function rgb([r, g, b]: [number, number, number]): string {
    return `rgb(${r}, ${g}, ${b})`;
  }
</script>

<h2>players <span class="dim">({players.length} connected)</span></h2>

{#if error}<p class="danger">{error}</p>{/if}

{#if players.length === 0}
  <p class="dim">nobody is connected.</p>
{:else}
  <table>
    <thead>
      <tr>
        <th></th>
        <th>name</th>
        <th>health</th>
        <th>mana</th>
        <th>position</th>
        <th>address</th>
        <th>pvp</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each players as p (p.slot)}
        <tr>
          <td>
            {#if p.appearance}
              <span class="swatch" style:background={rgb(p.appearance.skin_color)}></span>
            {/if}
          </td>
          <td>{p.name}</td>
          <td>{health(p)}</td>
          <td>{p.mana} / {p.mana_max}</td>
          <td class="dim">{Math.round(p.x / 16)}, {Math.round(p.y / 16)}</td>
          <td class="dim">{p.address}</td>
          <td>{p.pvp ? "on" : "off"}</td>
          <td class="actions">
            <button disabled={acting === p.name} onclick={() => kick(p)}>kick</button>
            <button class="danger-btn" disabled={acting === p.name} onclick={() => openBan(p)}>ban</button>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

{#if banTarget}
  <div
    class="overlay"
    role="presentation"
    onkeydown={(e) => e.key === "Escape" && (banTarget = null)}
  >
    <div class="dialog" role="dialog" aria-label="ban player" tabindex="-1">
      <h3>ban {banTarget.name}</h3>
      <label>
        kind
        <select bind:value={banKind}>
          <option value="name">name</option>
          <option value="ip">ip address</option>
          <option value="uuid">uuid</option>
        </select>
      </label>
      <label>
        value
        <input bind:value={banValue} />
      </label>
      <label>
        reason
        <input bind:value={banReason} placeholder="banned from the web panel" />
      </label>
      <div class="dialog-actions">
        <button onclick={() => (banTarget = null)}>cancel</button>
        <button class="danger-btn" onclick={confirmBan}>confirm ban</button>
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

  table {
    width: 100%;
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

  .swatch {
    display: inline-block;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1px solid var(--border);
  }

  .actions {
    display: flex;
    gap: 0.4rem;
    justify-content: flex-end;
  }

  .danger-btn {
    background: transparent;
    border-color: var(--danger);
    color: var(--danger);
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
    border-radius: 4px;
    padding: 1.25rem;
    width: 320px;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .dialog h3 {
    margin: 0;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 2px;
    padding: 0.4rem;
    font-family: inherit;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }
</style>
