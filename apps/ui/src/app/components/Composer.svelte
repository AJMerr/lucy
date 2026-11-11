<!-- Where you type to Lucy: Enter sends, Shift+Enter starts a new line, and while she's
     replying, Stop cuts it short. Pictures pasted, dropped or attached go with the message. -->
<script lang="ts">
  import { type Attached, PICTURE_TYPES, pictureUrl, readPicture } from "../../lib/pictures.ts";
  import type { ImageData } from "../../lib/protocol.ts";

  let {
    busy,
    onSend,
    onStop,
    placeholder = "Message Lucy… (Enter sends, Shift+Enter for a new line)",
    talk,
  }: {
    busy: boolean;
    onSend: (text: string, images: ImageData[]) => void;
    onStop: () => void;
    placeholder?: string;
    /** Voice is on: the mic button starts (`hold`: until `stop`) and stops listening. */
    talk?: { start: (hold: boolean) => void; stop: () => void };
  } = $props();

  /** A press shorter than this is a click: listen until you go quiet instead. */
  const CLICK_MS = 300;
  let pressedAt = 0;

  function micDown(): void {
    pressedAt = Date.now();
    talk?.start(true);
  }

  function micUp(): void {
    if (Date.now() - pressedAt < CLICK_MS) {
      talk?.start(false);
    } else {
      talk?.stop();
    }
  }

  let text = $state("");
  let attached: Attached[] = $state([]);
  let problem = $state("");
  let box: HTMLTextAreaElement | undefined = $state();
  let chooser: HTMLInputElement | undefined = $state();

  function send(): void {
    const message = text.trim();
    if (message === "" && attached.length === 0) {
      return;
    }
    onSend(
      message,
      attached.map(({ media_type, data }) => ({ media_type, data })),
    );
    text = "";
    attached = [];
    problem = "";
    fit();
  }

  /** Attach the pictures among `files`; say why any others were turned away. */
  export async function add(files: Iterable<File>): Promise<void> {
    problem = "";
    for (const file of files) {
      try {
        attached = [...attached, await readPicture(file)];
      } catch (err) {
        problem = err instanceof Error ? err.message : String(err);
      }
    }
  }

  function paste(event: ClipboardEvent): void {
    const files = [...(event.clipboardData?.files ?? [])];
    if (files.length > 0) {
      event.preventDefault();
      void add(files);
    }
  }

  function drop(event: DragEvent): void {
    const files = [...(event.dataTransfer?.files ?? [])];
    if (files.length > 0) {
      event.preventDefault();
      event.stopPropagation();
      void add(files);
    }
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
  ondragover={(event) => event.preventDefault()}
  ondrop={drop}
>
  {#if attached.length > 0 || problem !== ""}
    <div class="attached">
      {#each attached as picture, i (i)}
        <span class="picture">
          <img src={pictureUrl(picture)} alt={picture.name} />
          <button
            type="button"
            class="remove"
            aria-label={`Remove ${picture.name}`}
            onclick={() => (attached = attached.filter((_, at) => at !== i))}>×</button
          >
        </span>
      {/each}
      {#if problem !== ""}
        <span class="problem">{problem}</span>
      {/if}
    </div>
  {/if}
  <div class="row">
    <button
      type="button"
      class="attach"
      aria-label="Attach a picture"
      title="Attach a picture (or paste or drop one)"
      onclick={() => chooser?.click()}>+</button
    >
    {#if talk}
      <button
        type="button"
        class="attach"
        aria-label="Talk (hold, or click)"
        title="Hold to talk, or click and talk until you pause"
        onpointerdown={micDown}
        onpointerup={micUp}>🎙</button
      >
    {/if}
    <input
      bind:this={chooser}
      type="file"
      accept={PICTURE_TYPES.join(",")}
      multiple
      hidden
      onchange={(event) => {
        const input = event.currentTarget;
        void add([...(input.files ?? [])]).then(() => (input.value = ""));
      }}
    />
    <textarea
      bind:this={box}
      bind:value={text}
      rows="1"
      {placeholder}
      onkeydown={keydown}
      oninput={fit}
      onpaste={paste}
    ></textarea>
    {#if busy}
      <button type="button" onclick={onStop}>Stop</button>
    {/if}
    <button class="primary" type="submit">Send</button>
  </div>
</form>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--line);
    background: var(--panel);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .attached {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .picture {
    position: relative;
  }
  .picture img {
    display: block;
    height: 64px;
    max-width: 120px;
    object-fit: cover;
    border: 1px solid var(--line);
    border-radius: 6px;
  }
  .remove {
    position: absolute;
    top: -6px;
    right: -6px;
    width: 20px;
    height: 20px;
    padding: 0;
    border-radius: 50%;
    line-height: 1;
  }
  .problem {
    color: var(--hot);
    font-size: 12px;
  }
  .attach {
    font-size: 18px;
    line-height: 1;
    padding: 0 10px;
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
