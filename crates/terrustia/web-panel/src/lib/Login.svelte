<script lang="ts">
  import { login, ApiCallError } from "./api";

  let { unclaimed, onLoggedIn }: { unclaimed: boolean; onLoggedIn: () => void } = $props();

  let name = $state("");
  let password = $state("");
  let claimToken = $state("");
  let error = $state("");
  let busy = $state(false);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    error = "";
    busy = true;
    try {
      await login({ name, password, claim_token: unclaimed ? claimToken : undefined });
      onLoggedIn();
    } catch (e) {
      error = e instanceof ApiCallError ? e.message : "something went wrong";
    } finally {
      busy = false;
    }
  }
</script>

<div class="wrap">
  <form onsubmit={submit}>
    <h1>terrustia</h1>
    {#if unclaimed}
      <p class="dim">
        This server hasn't been claimed yet. Enter the token printed to the server's own console —
        the same one <code>/register</code> uses — to create the first account.
      </p>
    {:else}
      <p class="dim">Sign in with your server account.</p>
    {/if}

    <label>
      name
      <input bind:value={name} autocomplete="username" required />
    </label>
    <label>
      password
      <input bind:value={password} type="password" autocomplete="current-password" required />
    </label>
    {#if unclaimed}
      <label>
        claim token
        <input bind:value={claimToken} required />
      </label>
    {/if}

    {#if error}
      <p class="danger">{error}</p>
    {/if}

    <button type="submit" disabled={busy}>{busy ? "signing in…" : unclaimed ? "claim" : "sign in"}</button>
  </form>
</div>

<style>
  .wrap {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1rem;
  }

  form {
    width: min(360px, 100%);
    border: 1px solid var(--border);
    background: var(--bg-raised);
    border-radius: var(--radius-lg);
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  h1 {
    margin: 0 0 0.25rem;
    font-size: 1.1rem;
    letter-spacing: 0.02em;
  }

  p {
    margin: 0 0 0.25rem;
  }

  code {
    color: var(--text);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  button {
    margin-top: 0.5rem;
  }
</style>
