<!-- Lucy's main window: the chat list beside the open chat. On narrow screens (and phones) the
     list folds into a menu. -->
<script lang="ts">
  import { onMount, tick } from "svelte";

  import type { ConnectionState, Transport } from "../lib/rpc/transport.ts";
  import { ChatsStore, type UndoResult } from "../lib/stores/chats.ts";
  import { SettingsStore } from "../lib/stores/settings.ts";
  import { TasksStore } from "../lib/stores/tasks.ts";
  import FirstRun from "./FirstRun.svelte";
  import ChatHeader, { type ProfileInfo } from "./components/ChatHeader.svelte";
  import Composer from "./components/Composer.svelte";
  import Entry from "./components/Entry.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import Settings from "./settings/Settings.svelte";
  import Tasks from "./tasks/Tasks.svelte";

  let { core }: { core: Transport } = $props();

  // svelte-ignore state_referenced_locally
  const chats = new ChatsStore(core);
  // svelte-ignore state_referenced_locally
  const settings = new SettingsStore(core);
  // svelte-ignore state_referenced_locally
  const tasks = new TasksStore(core);
  /** What the window shows: chats, settings, or first-run setup (no config yet). */
  let view: "chat" | "settings" | "tasks" | "setup" = $state("chat");
  const desktop = "__TAURI_INTERNALS__" in window;

  /** Run `lucy <args>` on this machine; only the desktop app can. */
  async function runLucy(args: string[]): Promise<string> {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<string>("run_lucy", { args });
  }
  let profiles: ProfileInfo[] = $state([]);
  let connection: ConnectionState = $state("connecting");
  let problem: string | null = $state(null);
  let menuOpen = $state(false);
  let undoing: UndoResult | null = $state(null);
  let log: HTMLElement | undefined = $state();
  let composer: Composer | undefined = $state();

  const selected = $derived($chats.selected);
  const chat = $derived(selected === null ? undefined : $chats.chats[selected]);
  const session = $derived($chats.sessions.find((s) => s.id === selected));
  const unread = $derived(
    new Set(
      Object.entries($chats.chats)
        .filter(([, c]) => c.unread)
        .map(([id]) => id),
    ),
  );
  const thinking = $derived.by(() => {
    if (chat === undefined || !chat.busy) {
      return false;
    }
    const last = chat.transcript.entries.at(-1);
    return !(last?.kind === "lucy" && last.streaming) && !(last?.kind === "tool" && last.state === "asking");
  });

  onMount(() => {
    connection = core.state;
    const stop = core.onState((state) => {
      connection = state;
      if (state === "ready") {
        refresh();
      }
    });
    refresh();
    // The desktop app asks for a chat by id ("more…" on the sprite's bubble).
    let unlisten: (() => void) | undefined;
    if ("__TAURI_INTERNALS__" in window) {
      import("@tauri-apps/api/event").then(({ listen }) =>
        listen<string>("app-open-chat", (event) => open(event.payload)).then((fn) => (unlisten = fn)),
      );
      // Opened by "more…": the chat it was opened for.
      import("@tauri-apps/api/core").then(({ invoke }) =>
        invoke<string | null>("take_pending_chat").then((id) => {
          if (id !== null) {
            open(id);
          }
        }),
      );
    }
    return () => {
      stop();
      unlisten?.();
    };
  });

  function refresh(): void {
    // No config yet: first-run setup instead of chats.
    core.call<{ config: unknown }>("config.get").then(
      (got) => {
        if (got.config === null) {
          view = "setup";
        } else if (view === "setup") {
          view = "chat";
        }
      },
      () => {},
    );
    chats.load().then(
      () => (problem = null),
      (err: unknown) => (problem = message(err)),
    );
    core.call<ProfileInfo[]>("profiles.list").then(
      (list) => (profiles = list),
      () => {},
    );
  }

  async function open(id: string): Promise<void> {
    menuOpen = false;
    undoing = null;
    try {
      await chats.open(id);
      await tick();
      scrollDown(true);
    } catch (err) {
      problem = message(err);
    }
  }

  async function startUndo(): Promise<void> {
    try {
      undoing = await chats.undo(false);
    } catch (err) {
      problem = message(err);
    }
  }

  async function finishUndo(): Promise<void> {
    try {
      await chats.undo(true);
    } catch (err) {
      problem = message(err);
    }
    undoing = null;
  }

  /** Keep the newest message in view, unless you've scrolled up to read. */
  function scrollDown(force = false): void {
    if (log === undefined) {
      return;
    }
    const near = log.scrollHeight - log.scrollTop - log.clientHeight < 80;
    if (force || near) {
      log.scrollTop = log.scrollHeight;
    }
  }

  $effect.pre(() => {
    void chat?.transcript;
    tick().then(() => scrollDown());
  });

  function message(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }
</script>

{#if view === "setup"}
  <FirstRun
    {core}
    onDone={() => {
      view = "chat";
      refresh();
    }}
  />
{:else if view === "tasks"}
  <div class="page">
    <Tasks
      store={tasks}
      profiles={profiles.map((p) => p.name)}
      onClose={() => (view = "chat")}
    />
  </div>
{:else if view === "settings"}
  <div class="page">
    <Settings
      store={settings}
      runLucy={desktop ? runLucy : undefined}
      onClose={() => {
        view = "chat";
        refresh();
      }}
    />
  </div>
{:else}
<div class="shell" class:menu-open={menuOpen}>
  <aside>
    <Sidebar
      sessions={$chats.sessions}
      {selected}
      {unread}
      onSelect={open}
      onNewIncognito={() => {
        chats.newChat(true).catch((err) => (problem = message(err)));
        menuOpen = false;
      }}
      onNew={() => {
        chats.newChat().catch((err) => (problem = message(err)));
        menuOpen = false;
      }}
      onRename={(id, title) => chats.rename(id, title).catch((err) => (problem = message(err)))}
      onDelete={(id) => chats.remove(id).catch((err) => (problem = message(err)))}
      onSettings={() => (view = "settings")}
      onTasks={() => {
        view = "tasks";
        tasks.load().catch((err) => (problem = message(err)));
      }}
    />
  </aside>

  <main
    ondragover={(event) => event.preventDefault()}
    ondrop={(event) => {
      // A picture dropped anywhere on the chat goes with your next message.
      const files = [...(event.dataTransfer?.files ?? [])];
      if (files.length > 0) {
        event.preventDefault();
        void composer?.add(files);
      }
    }}
  >
    <button class="menu" aria-label="Chats" onclick={() => (menuOpen = !menuOpen)}>☰</button>
    {#if connection === "lost"}
      <div class="banner">Lost the connection to Lucy's core. Trying again…</div>
    {/if}
    {#if problem !== null}
      <div class="banner">
        {problem}
        <button class="link" onclick={() => (problem = null)}>Dismiss</button>
      </div>
    {/if}

    {#if chat !== undefined && session !== undefined}
      <ChatHeader
        title={session.title ?? "Untitled chat"}
        incognito={session.incognito === true}
        settings={chat.settings}
        {profiles}
        busy={chat.busy}
        onProfile={(name) => chats.setProfile(name)}
        onMode={(mode) => chats.setMode(mode)}
        onLevel={(level) => chats.setLevel(level)}
        onPlain={(plain) => chats.setPlain(plain)}
        onUndo={startUndo}
      />
      {#if undoing !== null}
        <div class="undo">
          {#if undoing.files.length === 0}
            Nothing to undo: her last reply didn't change any files.
            <button onclick={() => (undoing = null)}>OK</button>
          {:else}
            Put these back as they were before her last reply?
            <ul>
              {#each undoing.files as file (file.path)}
                <li><code>{file.path}</code>{file.created ? " (new file, removed)" : ""}</li>
              {/each}
            </ul>
            <button class="primary" onclick={finishUndo}>Undo</button>
            <button onclick={() => (undoing = null)}>Keep them</button>
          {/if}
        </div>
      {/if}
      <section class="log" bind:this={log}>
        {#each chat.transcript.entries as entry, i (i)}
          <Entry
            {entry}
            onDecide={(callId, decision) => chats.approve(callId, decision)}
            picture={(id) => chats.picture(id)}
          />
        {/each}
        {#if thinking}
          <div class="thinking" aria-label="Lucy is thinking">…</div>
        {/if}
      </section>
      <Composer
        bind:this={composer}
        busy={chat.busy}
        onSend={(text, images) => chats.send(text, images)}
        onStop={() => chats.cancel()}
      />
    {:else}
      <section class="welcome">
        <h1>Lucy</h1>
        {#if $chats.incognito}
          <p>Incognito chat: she won't read or save memories, and it's deleted when you leave it.</p>
        {:else}
          <p>Pick a chat, or start a new one below.</p>
        {/if}
      </section>
      <Composer
        bind:this={composer}
        busy={false}
        placeholder="Start a new chat… (Enter sends)"
        onSend={(text, images) => chats.send(text, images)}
        onStop={() => {}}
      />
    {/if}
  </main>
</div>
{/if}

<style>
  .page {
    height: 100vh;
  }
  .shell {
    display: grid;
    grid-template-columns: 260px 1fr;
    height: 100vh;
  }
  aside {
    background: var(--sidebar);
    border-right: 1px solid var(--line);
    min-height: 0;
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    position: relative;
  }
  .menu {
    display: none;
  }
  .log {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .thinking {
    align-self: flex-start;
    color: var(--muted);
  }
  .welcome {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    color: var(--muted);
  }
  .welcome h1 {
    color: var(--accent);
    font-weight: 600;
    letter-spacing: 0.04em;
    margin: 0 0 4px;
  }
  .banner {
    padding: 8px 16px;
    background: rgba(255, 79, 163, 0.12);
    border-bottom: 1px solid var(--hot);
    font-size: 13px;
  }
  .undo {
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
    font-size: 13px;
  }
  .undo ul {
    margin: 6px 0;
    padding-left: 20px;
  }

  /* Narrow windows and phones: the chat list becomes a menu. */
  @media (max-width: 700px) {
    .shell {
      grid-template-columns: 1fr;
    }
    aside {
      position: fixed;
      inset: 0 30% 0 0;
      z-index: 2;
      transform: translateX(-100%);
      transition: transform 0.15s ease;
    }
    .menu-open aside {
      transform: none;
    }
    .menu {
      display: block;
      position: absolute;
      top: 8px;
      left: 8px;
      z-index: 1;
      background: none;
      border: none;
      font-size: 18px;
    }
    main :global(header) {
      padding-left: 44px;
    }
  }
</style>
