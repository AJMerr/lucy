<!-- Where you type to Lucy: Enter sends, Shift+Enter starts a new line, and while she's
     replying, Stop cuts it short. -->
<script lang="ts">
  let {
    busy,
    onSend,
    onStop,
    placeholder = "Message Lucy… (Enter sends, Shift+Enter for a new line)",
  }: {
    busy: boolean;
    onSend: (text: string) => void;
    onStop: () => void;
    placeholder?: string;
  } = $props();

  let text = $state("");
  let box: HTMLTextAreaElement | undefined = $state();

  function send(): void {
    const message = text.trim();
    if (message === "") {
      return;
    }
    onSend(message);
    text = "";
    fit();
  }

  function keydown(event: KeyboardEvent): void {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      send();
    }
  }

  /** Grow with what you type, up to the box's max height. */
  function fit(): void {
    if (box === undefined) {
      return;
    }
    box.style.height = "auto";
    box.style.height = `${box.scrollHeight + 2}px`;
  }

  export function focus(): void {
    box?.focus();
  }
</script>

<form
  onsubmit={(event) => {
    event.preventDefault();
    send();
  }}
>
  <textarea
    bind:this={box}
    bind:value={text}
    rows="1"
    {placeholder}
    onkeydown={keydown}
    oninput={fit}
  ></textarea>
  {#if busy}
    <button type="button" onclick={onStop}>Stop</button>
  {/if}
  <button class="primary" type="submit">Send</button>
</form>

<style>
  form {
    display: flex;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--line);
    background: var(--panel);
  }
  textarea {
    flex: 1;
    resize: none;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 7px 10px;
    max-height: 30vh;
  }
  textarea:focus {
    outline: 1px solid var(--accent);
  }
</style>
