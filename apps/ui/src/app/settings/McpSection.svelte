<!-- MCP servers: tool servers Lucy starts as programs or reaches by URL, with their settings,
     variables and headers (some filled from the keyring), and sign-in for remote ones. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import SectionBar from "./SectionBar.svelte";

  let { store }: { store: SettingsStore } = $props();

  const section = $derived($store.sections.mcp);
  const servers = $derived(section.draft.mcp?.servers ?? {});
  const names = $derived(Object.keys(servers).sort());

  let newName = $state("");
  let signIn: Record<string, string> = $state({});

  function set(server: string, field: string, value: unknown): void {
    store.edit("mcp", (d) => {
      const empty =
        value === "" ||
        value === undefined ||
        (Array.isArray(value) && value.length === 0) ||
        (typeof value === "object" && value !== null && Object.keys(value).length === 0);
      if (empty) {
        delete d.mcp.servers[server][field];
      } else {
        d.mcp.servers[server][field] = value;
      }
    });
  }

  function add(kind: "command" | "url"): void {
    const name = newName.trim();
    if (name === "" || servers[name] !== undefined) {
      return;
    }
    store.edit("mcp", (d) => {
      d.mcp.servers[name] = { [kind]: "" };
    });
    newName = "";
  }

  function remove(server: string): void {
    store.edit("mcp", (d) => {
      delete d.mcp.servers[server];
    });
  }

  /** `NAME=value` lines to and from a table. */
  const lines = (table: Record<string, string> | undefined) =>
    Object.entries(table ?? {})
      .map(([k, v]) => `${k}=${v}`)
      .join("\n");
  function table(text: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of text.split("\n")) {
      const at = line.indexOf("=");
      if (at > 0) {
        out[line.slice(0, at).trim()] = line.slice(at + 1).trim();
      }
    }
    return out;
  }

  async function login(server: string): Promise<void> {
    signIn[server] = "Finish signing in in your browser…";
    try {
      await store.signIn(server);
      signIn[server] = "Signed in. New chats can use its tools.";
    } catch (err) {
      signIn[server] = err instanceof Error ? err.message : String(err);
    }
  }
</script>

<h2>MCP servers</h2>
<p class="hint">
  Tool servers Lucy can use: programs she starts, or services she reaches by URL. Their tools
  follow the same approval rules as hers. Keys named here are set under Models &amp; keys.
</p>

{#each names as name (name)}
  {@const server = servers[name]}
  {@const remote = server.url !== undefined}
  <fieldset aria-label={name}>
    <legend>{name}</legend>
    <label class="check">
      <input
        type="checkbox"
        aria-label="On"
        checked={server.enabled !== false}
        onchange={(e) => set(name, "enabled", e.currentTarget.checked ? undefined : false)}
      />
      On
    </label>
    {#if remote}
      <label class="field">
        <span>URL</span>
        <input aria-label="URL" value={server.url ?? ""} oninput={(e) => set(name, "url", e.currentTarget.value.trim())} />
      </label>
      <label class="field">
        <span>Bearer token key name (keyring)</span>
        <input
          value={server.bearer_key ?? ""}
          placeholder="none: sign in instead, or no auth"
          oninput={(e) => set(name, "bearer_key", e.currentTarget.value.trim())}
        />
      </label>
      <label class="field">
        <span>Headers, one per line: Name=value</span>
        <textarea rows="2" value={lines(server.headers)} oninput={(e) => set(name, "headers", table(e.currentTarget.value))}></textarea>
      </label>
      <label class="field">
        <span>Headers from the keyring, one per line: Name=key name</span>
        <textarea
          rows="2"
          value={lines(server.header_keys)}
          oninput={(e) => set(name, "header_keys", table(e.currentTarget.value))}
        ></textarea>
      </label>
      <div class="row">
        <button aria-label={`Sign in to ${name}`} onclick={() => login(name)}>Sign in</button>
        {#if signIn[name]}<span class="muted">{signIn[name]}</span>{/if}
      </div>
    {:else}
      <label class="field">
        <span>Command</span>
        <input
          aria-label="Command"
          value={server.command ?? ""}
          placeholder="e.g. npx"
          oninput={(e) => set(name, "command", e.currentTarget.value.trim())}
        />
      </label>
      <label class="field">
        <span>Arguments, one per line</span>
        <textarea
          aria-label="Arguments, one per line"
          rows="2"
          value={(server.args ?? []).join("\n")}
          oninput={(e) =>
            set(
              name,
              "args",
              e.currentTarget.value
                .split("\n")
                .map((s) => s.trim())
                .filter(Boolean),
            )}
        ></textarea>
      </label>
      <label class="field">
        <span>Folder to run in</span>
        <input value={server.cwd ?? ""} oninput={(e) => set(name, "cwd", e.currentTarget.value.trim())} />
      </label>
      <label class="field">
        <span>Variables, one per line: NAME=value</span>
        <textarea rows="2" value={lines(server.env)} oninput={(e) => set(name, "env", table(e.currentTarget.value))}></textarea>
      </label>
      <label class="field">
        <span>Variables from the keyring, one per line: NAME=key name</span>
        <textarea
          rows="2"
          value={lines(server.env_keys)}
          oninput={(e) => set(name, "env_keys", table(e.currentTarget.value))}
        ></textarea>
      </label>
    {/if}
    <button class="link danger-text" aria-label={`Remove ${name}`} onclick={() => remove(name)}>
      Remove {name}
    </button>
  </fieldset>
{:else}
  <p class="muted">No MCP servers yet.</p>
{/each}

<div class="row add">
  <input aria-label="New server name" placeholder="New server name" bind:value={newName} />
  <button onclick={() => add("command")}>Add a program</button>
  <button onclick={() => add("url")}>Add a URL</button>
</div>

<SectionBar
  {section}
  managed={$store.managed}
  onSave={() => store.save("mcp")}
  onRevert={() => store.revert("mcp")}
/>

<style>
  .add {
    margin: 8px 0;
  }
  textarea {
    resize: vertical;
  }
</style>
