<script lang="ts">
  import { onMount } from "svelte";
  import { storedSession, fetchStatus, fetchUnclaimed, ApiCallError } from "./lib/api";
  import Login from "./lib/Login.svelte";
  import Status from "./lib/Status.svelte";

  type ViewState =
    | { kind: "loading" }
    | { kind: "login"; unclaimed: boolean }
    | { kind: "session"; session: string };

  let view = $state<ViewState>({ kind: "loading" });

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
        view = { kind: "login", unclaimed: await fetchUnclaimed() };
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
</style>
