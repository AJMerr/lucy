<!-- One entry in a chat: your message, her reply (light Markdown, always escaped), or a tool
     call, with Allow / Always / Deny while it waits for you. -->
<script lang="ts">
  import { renderBubble } from "../../lib/bubble.ts";
  import type { Entry } from "../../lib/transcript.ts";

  type Decision = "once" | "always" | "deny";

  let { entry, onDecide }: { entry: Entry; onDecide: (callId: string, decision: Decision) => void } =
    $props();

  let showChanges = $state(false);

  // renderBubble escapes everything and adds only its own tags, so this is safe as HTML.
  const html = $derived(
    entry.kind === "lucy" && entry.text !== ""
      ? renderBubble(entry.text, { maxLines: 0, charsPerLine: 80, expanded: true }).html
      : "",
  );

  /** "run `ls`" as text and code parts; the model chose it, so never HTML. */
  function parts(summary: string): { code: boolean; text: string }[] {
    return summary.split("`").map((text, i) => ({ code: i % 2 === 1, text }));
  }
</script>

{#if entry.kind === "you"}
  <div class="you">{entry.text}</div>
{:else if entry.kind === "lucy"}
  <div class="lucy" class:streaming={entry.streaming}>
    {#if html !== ""}
      <div class="markdown">{@html html}</div>
    {/if}
    {#if entry.error !== undefined}
      <div class="error">{entry.error}</div>
    {/if}
    {#if entry.note !== undefined}
      <div class="note">({entry.note})</div>
    {/if}
  </div>
{:else if entry.state === "asking"}
  <div class="tool asking">
    <div>
      Okay to {#each parts(entry.summary) as part}{#if part.code}<code>{part.text}</code
          >{:else}{part.text}{/if}{/each}?
    </div>
    {#if entry.reason}
      <div class="reason">{entry.reason}</div>
    {/if}
    {#if entry.preview !== undefined && showChanges}
      <pre>{entry.preview}</pre>
    {/if}
    <div class="buttons">
      <button class="primary" onclick={() => onDecide(entry.callId, "once")}>Allow</button>
      <button onclick={() => onDecide(entry.callId, "always")}>Always</button>
      <button onclick={() => onDecide(entry.callId, "deny")}>Deny</button>
      {#if entry.preview !== undefined}
        <button class="link" onclick={() => (showChanges = !showChanges)}>
          {showChanges ? "Hide changes" : "Show changes"}
        </button>
      {/if}
    </div>
  </div>
{:else}
  <div class="tool" class:failed={entry.state === "failed"}>
    ▸ {#each parts(entry.summary) as part}{#if part.code}<code>{part.text}</code
        >{:else}{part.text}{/if}{/each}
    {#if entry.state === "running"}<span class="state"> · running</span>{/if}
    {#if entry.state === "failed"}<span class="state"> · failed</span>{/if}
  </div>
{/if}

<style>
  .you {
    align-self: flex-end;
    max-width: 80%;
    padding: 8px 12px;
    background: var(--you-bg);
    border: 1px solid var(--line);
    border-radius: 12px 12px 2px 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .lucy {
    align-self: flex-start;
    max-width: 92%;
    padding: 8px 12px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 12px 12px 12px 2px;
    overflow-wrap: anywhere;
  }
  .lucy.streaming {
    border-color: var(--accent-dim);
  }
  .markdown :global(p),
  .markdown :global(ul),
  .markdown :global(ol) {
    margin: 0 0 6px;
  }
  .markdown :global(:last-child) {
    margin-bottom: 0;
  }
  .markdown :global(ul),
  .markdown :global(ol) {
    padding-left: 20px;
  }
  .error {
    color: var(--hot);
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .tool {
    align-self: flex-start;
    color: var(--muted);
    font-size: 13px;
  }
  .tool.failed .state {
    color: var(--hot);
  }
  .tool.asking {
    color: var(--text);
    padding: 8px 12px;
    border: 1px solid var(--hot);
    border-radius: 10px;
    max-width: 92%;
  }
  .reason {
    color: var(--muted);
    font-size: 12px;
  }
  .buttons {
    display: flex;
    gap: 6px;
    margin-top: 6px;
    flex-wrap: wrap;
  }
</style>
