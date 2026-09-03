<script lang="ts">
  import { onMount } from "svelte";
  import { fetchConfig, setMotd, type ConfigSnapshot } from "./api";

  let { session }: { session: string } = $props();

  let config = $state<ConfigSnapshot | null>(null);
  let error = $state("");
  let motdDraft = $state("");
  let saving = $state(false);
  let saved = $state(false);

  async function refresh() {
    try {
      config = await fetchConfig(session);
      motdDraft = config.motd;
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(refresh);

  async function saveMotd() {
    saving = true;
    saved = false;
    try {
      await setMotd(session, motdDraft);
      if (config) config.motd = motdDraft;
      saved = true;
      // A save that follows a failed one must not leave the old error sitting next to the new
      // "saved." — without this, a failure followed by a success showed both at once.
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  // "saved." otherwise never clears: it stayed shown under a draft the operator had since edited
  // further, which reads as "your new changes are saved" when they are not.
  function onMotdInput() {
    saved = false;
  }
</script>

<h2>settings</h2>

{#if error}<p class="danger">{error}</p>{/if}

{#if config}
  <section>
    <h3 class="dim">message of the day</h3>
    <p class="dim">
      the only setting this panel can change live — it is read fresh from the config every time a
      player joins, so a new value here takes effect for the very next join. everything below is
      read-only: it reflects the config file and command-line flags the process was started with.
    </p>
    <form class="motd-form" onsubmit={(e) => { e.preventDefault(); saveMotd(); }}>
      <input bind:value={motdDraft} oninput={onMotdInput} aria-label="message of the day" />
      <button disabled={saving}>{saving ? "saving…" : "save"}</button>
    </form>
    {#if saved}<p class="accent-text">saved.</p>{/if}
  </section>

  <section class="grid">
    <div class="card">
      <span class="label">listen address</span>
      <span class="value">{config.listen}</span>
    </div>
    <div class="card">
      <span class="label">max players</span>
      <span class="value">{config.max_players}</span>
    </div>
    <div class="card">
      <span class="label">world size</span>
      <span class="value">{config.world_width} × {config.world_height}</span>
    </div>
    <div class="card">
      <span class="label">password</span>
      <span class="value">{config.password_set ? "set" : "none — open server"}</span>
    </div>
    <div class="card">
      <span class="label">max chat length</span>
      <span class="value">{config.max_chat_len}</span>
    </div>
    <div class="card">
      <span class="label">idle timeout</span>
      <span class="value">{config.idle_timeout_secs}s</span>
    </div>
    <div class="card">
      <span class="label">autosave</span>
      <span class="value">
        {config.autosave_secs > 0 ? `every ${config.autosave_secs}s` : "disabled"}
      </span>
    </div>
    <div class="card">
      <span class="label">save destination</span>
      <span class="value small">{config.save_target ?? "none — this world will not be saved"}</span>
    </div>
    <div class="card">
      <span class="label">whitelist</span>
      <span class="value">
        {config.whitelist_on ? `on (${config.whitelist_count})` : "off"}
      </span>
    </div>
  </section>
{:else}
  <p class="dim">loading…</p>
{/if}

<style>
  h2 {
    margin: 0 0 1rem;
    font-size: 1rem;
    font-weight: 600;
  }

  h3 {
    margin: 0 0 0.4rem;
    font-size: 0.8rem;
    letter-spacing: 0.06em;
    font-weight: 500;
  }

  section {
    margin-bottom: 1.75rem;
  }

  section > p {
    max-width: 46em;
  }

  .accent-text {
    color: var(--accent);
  }

  .motd-form {
    display: flex;
    gap: 0.5rem;
    max-width: 520px;
  }

  .motd-form input {
    flex: 1;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 0.75rem;
    max-width: 800px;
  }

  .card {
    border: 1px solid var(--border);
    background: var(--bg-raised);
    border-radius: var(--radius-lg);
    padding: 0.9rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .label {
    font-size: 0.72rem;
    letter-spacing: 0.08em;
    color: var(--text-dim);
  }

  .value {
    font-size: 1.05rem;
    color: var(--accent);
    word-break: break-word;
  }

  .value.small {
    font-size: 0.82rem;
  }
</style>
