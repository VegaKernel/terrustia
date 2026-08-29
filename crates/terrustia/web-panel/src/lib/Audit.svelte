<script lang="ts">
  import { onMount } from "svelte";
  import { fetchAuditLog, type AuditEntry } from "./api";

  let { session }: { session: string } = $props();

  let entries = $state<AuditEntry[] | null>(null);
  let error = $state("");

  async function refresh() {
    try {
      entries = await fetchAuditLog(session, 100);
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(() => {
    refresh();
    const id = setInterval(refresh, 5000);
    return () => clearInterval(id);
  });

  function when(secs: number): string {
    if (secs === 0) return "unknown time";
    return new Date(secs * 1000).toLocaleString();
  }
</script>

<h2>audit log</h2>
<p class="dim">
  every ban, kick, mute, group change and permission edit, newest first. rotates automatically once
  the underlying file grows past its configured size — see <span class="mono">audit_log_max_bytes</span>
  in the server config.
</p>

{#if error}<p class="danger">{error}</p>{/if}

{#if entries}
  {#if entries.length === 0}
    <p class="dim">no audit events recorded yet.</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th>when</th>
          <th>issuer</th>
          <th>action</th>
          <th>target</th>
          <th>detail</th>
        </tr>
      </thead>
      <tbody>
        {#each [...entries].reverse() as e, i (i)}
          <tr>
            <td class="dim">{when(e.when)}</td>
            <td class="mono">{e.issuer || "—"}</td>
            <td><span class="action">{e.action}</span></td>
            <td class="mono">{e.target || "—"}</td>
            <td class="dim">{e.detail || "—"}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
{:else}
  <p class="dim">loading…</p>
{/if}

<style>
  h2 {
    margin: 0 0 0.4rem;
    font-size: 1rem;
    font-weight: 600;
  }

  p.dim {
    max-width: 640px;
    margin: 0 0 1rem;
    font-size: 0.82rem;
  }

  .mono {
    font-family: var(--mono);
    color: var(--text);
  }

  table {
    width: 100%;
    max-width: 900px;
    border-collapse: collapse;
    font-size: 0.85rem;
  }

  th,
  td {
    text-align: left;
    padding: 0.45rem 0.6rem;
    border-bottom: 1px solid var(--border);
  }

  th {
    color: var(--text-dim);
    font-weight: 500;
    text-transform: uppercase;
    font-size: 0.7rem;
    letter-spacing: 0.06em;
  }

  .action {
    display: inline-block;
    font-size: 0.72rem;
    color: var(--accent);
    border: 1px solid var(--accent-dim);
    border-radius: 2px;
    padding: 0.05rem 0.4rem;
  }
</style>
