<script lang="ts">
  import { tick } from "svelte";
  import { sendConsoleCommand, sendChat, type ConsoleFeedLine } from "./api";

  let { session, lines, live }: { session: string; lines: ConsoleFeedLine[]; live: boolean } =
    $props();

  type Mode = "chat" | "command";
  let mode = $state<Mode>("chat");
  let input = $state("");
  let sending = $state(false);
  let error = $state("");
  let logEl: HTMLDivElement | undefined;
  let stickToBottom = $state(true);

  $effect(() => {
    // Re-run whenever `lines` changes; only autoscroll if the viewer was already at the bottom,
    // so scrolling up to read history doesn't get yanked back down by the next line.
    void lines.length;
    if (stickToBottom && logEl) {
      tick().then(() => {
        if (logEl) logEl.scrollTop = logEl.scrollHeight;
      });
    }
  });

  function onScroll() {
    if (!logEl) return;
    const atBottom = logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 24;
    stickToBottom = atBottom;
  }

  async function send() {
    const text = input.trim();
    if (!text) return;
    sending = true;
    error = "";
    try {
      if (mode === "chat") await sendChat(session, text);
      else await sendConsoleCommand(session, text);
      input = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      sending = false;
    }
  }

  function kindClass(line: ConsoleFeedLine): string {
    if (line.line_kind === "chat") return "chat";
    if (line.line_kind === "reply") return "reply";
    if (line.level === "ERROR") return "level-error";
    if (line.level === "WARN") return "level-warn";
    return "";
  }
</script>

<div class="console">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -- a scrollable log region needs to be
       focusable to be scrolled with arrow keys, which is exactly what `role="log"` describes. -->
  <div class="log" bind:this={logEl} onscroll={onScroll} tabindex="0" role="log">
    {#if lines.length === 0}
      <p class="dim">
        {live ? "waiting for output…" : "not connected."} lines appear here from the moment this
        tab connects — no history from before it.
      </p>
    {/if}
    {#each lines as line, i (i)}
      <div class="line {kindClass(line)}">{line.text}</div>
    {/each}
  </div>

  <!-- Shown regardless of line count: once the log holds lines, a dropped socket used to look
       identical to a quiet server, since the disconnected message above only rendered while the
       log was still empty. -->
  {#if !live && lines.length > 0}<p class="warn">not connected  -  showing the last lines received.</p>{/if}

  {#if error}<p class="danger">{error}</p>{/if}

  <!-- The command placeholder below has to name commands the *console* actually runs. It used to
       suggest "time noon", which is a player slash command handled by `run_command`; the console's
       own `run_console` has no `time` arm, so typing the example this box gave you came straight
       back as `unknown command "time"`. `CONSOLE_HELP` is the list of record. -->
  <form class="send" onsubmit={(e) => { e.preventDefault(); send(); }}>
    <select bind:value={mode}>
      <option value="chat">chat</option>
      <option value="command">command</option>
    </select>
    <input
      placeholder={mode === "chat" ? "message to everyone in-game" : "e.g. players, save, help"}
      bind:value={input}
      disabled={sending}
    />
    <button disabled={sending || !input.trim()}>send</button>
  </form>
</div>

<style>
  .console {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    gap: 0.75rem;
  }

  .log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 0.75rem 0.9rem;
    font-size: 0.82rem;
  }

  .line {
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.6;
  }

  .line.chat {
    color: var(--text);
  }

  .line.reply {
    color: var(--accent);
  }

  .line.level-warn {
    color: var(--warn);
  }

  .line.level-error {
    color: var(--danger);
  }

  .send {
    display: flex;
    gap: 0.5rem;
  }

  .send select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.5rem;
    font-family: inherit;
  }

  .send input {
    flex: 1;
  }
</style>
