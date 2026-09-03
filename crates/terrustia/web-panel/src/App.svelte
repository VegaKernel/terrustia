<script lang="ts">
  import { onMount } from "svelte";
  import { storedSession, fetchStatus, fetchUnclaimed, ApiCallError } from "./lib/api";
  import Login from "./lib/Login.svelte";
  import Status from "./lib/Status.svelte";

  type ViewState =
    | { kind: "loading" }
    | { kind: "login"; unclaimed: boolean }
    | { kind: "session"; session: string }
    | { kind: "error"; message: string };

  let view = $state<ViewState>({ kind: "loading" });

  // A boot-time failure used to always drop to the login form, even for a session that was still
  // perfectly good: `unwrap` (`api.ts`) only clears the stored session on a real 401, so a 503
  // ("the game is not running") left a valid token in `localStorage` while the screen asked the
  // operator to sign in again — the actual problem (the game task is gone) was never named. The
  // stored session's presence after the call is now the signal: gone means 401, treat as signed
  // out; still there means something else failed, and that something is worth showing.
  async function checkSession() {
    const session = storedSession();
    if (!session) {
      view = { kind: "login", unclaimed: await fetchUnclaimed() };
      return;
    }
    try {
      await fetchStatus(session);
      view = { kind: "session", session };
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (storedSession()) {
          view = { kind: "error", message: e.message };
        } else {
          view = { kind: "login", unclaimed: await fetchUnclaimed() };
        }
      } else {
        throw e;
      }
    }
  }

  onMount(checkSession);
</script>

{#if view.kind === "loading"}
  <div class="loading dim">terrustia panel</div>
{:else if view.kind === "login"}
  <Login unclaimed={view.unclaimed} onLoggedIn={checkSession} />
{:else if view.kind === "error"}
  <div class="loading">
    <div class="boot-error">
      <p class="danger">{view.message}</p>
      <button onclick={checkSession}>retry</button>
    </div>
  </div>
{:else}
  <Status session={view.session} onLoggedOut={checkSession} />
{/if}

<style>
  .loading {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .boot-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-3);
    text-align: center;
  }
</style>
