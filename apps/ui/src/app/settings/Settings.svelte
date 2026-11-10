<!-- The settings page: a list of sections beside the open one. On narrow screens the list comes
     first and a section opens on its own, with a way back. -->
<script lang="ts">
  import { onMount } from "svelte";

  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import AgentSection from "./AgentSection.svelte";
  import ImagesSection from "./ImagesSection.svelte";
  import McpSection from "./McpSection.svelte";
  import ModelsSection from "./ModelsSection.svelte";
  import PersonaSection from "./PersonaSection.svelte";
  import RemoteSection from "./RemoteSection.svelte";
  import ServiceSection from "./ServiceSection.svelte";
  import SpriteSection from "./SpriteSection.svelte";

  let {
    store,
    runLucy,
    onClose,
  }: {
    store: SettingsStore;
    /** Run `lucy <args>` on this machine (desktop app only). */
    runLucy?: (args: string[]) => Promise<string>;
    onClose: () => void;
  } = $props();

  type Page = "models" | "agent" | "persona" | "mcp" | "images" | "sprite" | "remote" | "service";
  const pages: { id: Page; label: string }[] = $derived([
    { id: "models", label: "Models & keys" },
    { id: "agent", label: "Chat & agent" },
    { id: "persona", label: "Persona" },
    { id: "mcp", label: "MCP servers" },
    { id: "images", label: "Pictures" },
    { id: "sprite", label: "Sprite" },
    { id: "remote", label: "Remote access" },
    ...(runLucy ? [{ id: "service" as const, label: "Service & startup" }] : []),
  ]);

  let page: Page = $state("models");
  /** Narrow screens: showing a section rather than the list. */
  let reading = $state(false);
  let problem: string | null = $state(null);

  onMount(() => {
    store.load().catch((err: unknown) => (problem = err instanceof Error ? err.message : String(err)));
  });

  function open(id: Page): void {
    page = id;
    reading = true;
  }
</script>

<div class="settings" class:reading>
  <nav aria-label="Settings sections">
    <div class="top">
      <button class="link" onclick={onClose}>← Chats</button>
      <h1>Settings</h1>
    </div>
    {#each pages as entry (entry.id)}
      <button
        class="entry"
        aria-current={entry.id === page ? "page" : undefined}
        onclick={() => open(entry.id)}
      >
        {entry.label}
        {#if entry.id !== "persona" && entry.id !== "service" && $store.sections[entry.id]?.dirty}
          <span class="dot" aria-label="unsaved changes"></span>
        {/if}
      </button>
    {/each}
  </nav>

  <section>
    <button class="link back" onclick={() => (reading = false)}>← Settings</button>
    {#if problem !== null}<div class="error" role="alert">{problem}</div>{/if}
    {#if $store.managed}
      <div class="banner">
        config.toml is managed by home-manager (programs.lucy.settings), so settings can be looked at
        here but changed only in your Nix config. Keys and the persona still work.
      </div>
    {/if}
    {#if $store.restartNeeded}
      <div class="banner">
        Some changes apply once the core restarts{runLucy ? " (Service & startup → Restart)" : ""}.
      </div>
    {/if}
    {#if $store.loaded}
      {#if page === "models"}<ModelsSection {store} />
      {:else if page === "agent"}<AgentSection {store} />
      {:else if page === "persona"}<PersonaSection {store} />
      {:else if page === "mcp"}<McpSection {store} />
      {:else if page === "images"}<ImagesSection {store} />
      {:else if page === "sprite"}<SpriteSection {store} />
      {:else if page === "remote"}<RemoteSection
          {store}
          onToken={runLucy
            ? (rotate) => runLucy(rotate ? ["remote", "token", "--rotate"] : ["remote", "token"])
            : undefined}
        />
      {:else if page === "service" && runLucy}<ServiceSection run={runLucy} />
      {/if}
      <p class="where muted">Saved in {$store.path}</p>
    {:else}
      <p class="muted">Loading…</p>
    {/if}
  </section>
</div>

<style>
  .settings {
    display: grid;
    grid-template-columns: 220px 1fr;
    height: 100%;
    min-height: 0;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 8px;
    border-right: 1px solid var(--line);
    overflow-y: auto;
  }
  .top h1 {
    font-size: 16px;
    margin: 6px 10px 10px;
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: left;
    background: none;
    border: none;
    padding: 8px 10px;
    border-radius: 8px;
  }
  .entry[aria-current="page"] {
    background: var(--selected);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
  }
  section {
    overflow-y: auto;
    padding: 8px 24px 0;
    min-width: 0;
  }
  section :global(h2) {
    font-size: 18px;
    margin: 12px 0 6px;
  }
  section :global(h3) {
    font-size: 14px;
    margin: 20px 0 4px;
    color: var(--accent);
  }
  .banner {
    padding: 8px 12px;
    margin: 8px 0;
    border: 1px solid var(--hot);
    border-radius: 8px;
    font-size: 13px;
  }
  .where {
    margin: 16px 0;
  }
  .back {
    display: none;
  }

  @media (max-width: 700px) {
    .settings {
      grid-template-columns: 1fr;
    }
    section {
      display: none;
      padding: 8px 16px 0;
    }
    .reading nav {
      display: none;
    }
    .reading section {
      display: block;
    }
    .back {
      display: block;
    }
  }
</style>
