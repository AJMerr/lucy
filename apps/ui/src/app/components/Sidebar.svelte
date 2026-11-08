<!-- The list of chats: open one, start a new one, rename or delete (after asking). A dot marks
     chats with something new. -->
<script lang="ts">
  import type { SessionSummary } from "../../lib/stores/chats.ts";

  let {
    sessions,
    selected,
    unread,
    onSelect,
    onNew,
    onRename,
    onDelete,
  }: {
    sessions: SessionSummary[];
    selected: string | null;
    unread: Set<string>;
    onSelect: (id: string) => void;
    onNew: () => void;
    onRename: (id: string, title: string) => void;
    onDelete: (id: string) => void;
  } = $props();

  let renaming: string | null = $state(null);
  let draft = $state("");
  let confirming: string | null = $state(null);

  const title = (session: SessionSummary) => session.title ?? "Untitled chat";

  function startRename(session: SessionSummary): void {
    confirming = null;
    renaming = session.id;
    draft = session.title ?? "";
  }

  function renameKey(event: KeyboardEvent, id: string): void {
    if (event.key === "Enter") {
      event.preventDefault();
      const name = draft.trim();
      if (name !== "") {
        onRename(id, name);
      }
      renaming = null;
    } else if (event.key === "Escape") {
      renaming = null;
    }
  }
</script>

<nav>
  <button class="new primary" onclick={onNew}>New chat</button>
  <ul>
    {#each sessions as session (session.id)}
      <li aria-current={session.id === selected ? "true" : undefined}>
        {#if renaming === session.id}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            bind:value={draft}
            aria-label="New name"
            autofocus
            onkeydown={(event) => renameKey(event, session.id)}
            onblur={() => (renaming = null)}
          />
        {:else if confirming === session.id}
          <div class="confirm">
            Delete “{title(session)}”?
            <button class="danger" onclick={() => onDelete(session.id)}>Yes, delete</button>
            <button onclick={() => (confirming = null)}>Keep</button>
          </div>
        {:else}
          <button class="open" onclick={() => onSelect(session.id)}>
            <span class="title">{title(session)}</span>
            {#if unread.has(session.id)}<span class="dot" aria-label="new messages"></span>{/if}
          </button>
          <div class="actions">
            <button
              class="icon"
              aria-label={`Rename ${title(session)}`}
              title="Rename"
              onclick={() => startRename(session)}>✎</button
            >
            <button
              class="icon"
              aria-label={`Delete ${title(session)}`}
              title="Delete"
              onclick={() => {
                renaming = null;
                confirming = session.id;
              }}>🗑</button
            >
          </div>
        {/if}
      </li>
    {/each}
  </ul>
</nav>

<style>
  nav {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 8px;
    height: 100%;
    box-sizing: border-box;
    overflow-y: auto;
  }
  .new {
    margin: 0 4px 4px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  li {
    display: flex;
    align-items: center;
    border-radius: 8px;
  }
  li[aria-current="true"] {
    background: var(--selected);
  }
  li:hover .actions,
  li:focus-within .actions {
    opacity: 1;
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    text-align: left;
    background: none;
    border: none;
    padding: 8px 10px;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
  }
  .actions {
    display: flex;
    opacity: 0;
  }
  .icon {
    background: none;
    border: none;
    padding: 4px 6px;
    color: var(--muted);
  }
  input {
    flex: 1;
    margin: 4px;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--accent);
    border-radius: 6px;
    padding: 5px 8px;
  }
  .confirm {
    padding: 6px 10px;
    font-size: 13px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
</style>
