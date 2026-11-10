<!-- Models & keys: profiles (provider, endpoint, model, key name, prices), the default one, and
     the API keys they use, which go straight to the keyring and are never shown. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import SectionBar from "./SectionBar.svelte";

  let { store }: { store: SettingsStore } = $props();

  const section = $derived($store.sections.models);
  const draft = $derived(section.draft);
  const names = $derived(Object.keys(draft.profiles ?? {}).sort());

  let newName = $state("");
  /** Keys being typed, by key name. Cleared once sent. */
  let typed: Record<string, string> = $state({});
  let keyProblem: string | null = $state(null);

  function set(profile: string, field: string, value: unknown): void {
    store.edit("models", (d) => {
      if (value === "" || value === undefined || value === null) {
        delete d.profiles[profile][field];
      } else {
        d.profiles[profile][field] = value;
      }
    });
  }

  function add(): void {
    const name = newName.trim();
    if (name === "" || draft.profiles?.[name] !== undefined) {
      return;
    }
    store.edit("models", (d) => {
      d.profiles ??= {};
      d.profiles[name] = { provider: "openai_compat", model: "" };
    });
    newName = "";
  }

  function remove(profile: string): void {
    store.edit("models", (d) => {
      delete d.profiles[profile];
    });
  }

  async function setKey(name: string): Promise<void> {
    const value = (typed[name] ?? "").trim();
    if (value === "") {
      return;
    }
    typed[name] = "";
    try {
      keyProblem = null;
      await store.setKey(name, value);
    } catch (err) {
      keyProblem = err instanceof Error ? err.message : String(err);
    }
  }

  function where(source: string | null): string {
    if (source === null) {
      return "missing";
    }
    return source === "keyring" ? "set, in the keyring" : `set, from $${source}`;
  }

  const price = (value: string) => (value.trim() === "" ? undefined : Number(value));
</script>

<h2>Models &amp; keys</h2>
<p class="hint">
  Each profile is a model Lucy can use. New chats use the default one; a chat's header switches
  between them.
</p>

<label class="field">
  <span>Default profile</span>
  <select
    aria-label="Default profile"
    value={draft.defaults?.profile ?? ""}
    onchange={(e) => store.edit("models", (d) => (d.defaults.profile = e.currentTarget.value))}
  >
    {#each names as name (name)}<option value={name}>{name}</option>{/each}
  </select>
</label>
<div class="row">
  <label class="field">
    <span>New chats start in</span>
    <select
      value={draft.defaults?.mode ?? "assist"}
      onchange={(e) => store.edit("models", (d) => (d.defaults.mode = e.currentTarget.value))}
    >
      <option value="assist">Assist (asks before risky steps)</option>
      <option value="agent">Agent (works through tasks on her own)</option>
    </select>
  </label>
  <label class="field">
    <span>At agent level</span>
    <select
      value={String(draft.defaults?.agent_level ?? 1)}
      onchange={(e) =>
        store.edit("models", (d) => (d.defaults.agent_level = Number(e.currentTarget.value)))}
    >
      <option value="0">0 · ask for everything</option>
      <option value="1">1 · ask before risky steps</option>
      <option value="2">2 · free inside the workspace</option>
      <option value="3">3 · never ask</option>
    </select>
  </label>
</div>

{#each names as name (name)}
  {@const profile = draft.profiles[name]}
  <fieldset aria-label={name}>
    <legend>{name}</legend>
    <label class="field">
      <span>Provider</span>
      <select value={profile.provider} onchange={(e) => set(name, "provider", e.currentTarget.value)}>
        <option value="openai_compat">OpenAI-compatible (Ollama, OpenAI, OpenRouter, …)</option>
        <option value="anthropic">Anthropic</option>
      </select>
    </label>
    <label class="field">
      <span>Base URL</span>
      <input
        aria-label="Base URL"
        value={profile.base_url ?? ""}
        placeholder={profile.provider === "anthropic" ? "https://api.anthropic.com" : "http://localhost:11434/v1"}
        oninput={(e) => set(name, "base_url", e.currentTarget.value.trim())}
      />
    </label>
    <label class="field">
      <span>Model</span>
      <input aria-label="Model" value={profile.model ?? ""} oninput={(e) => set(name, "model", e.currentTarget.value)} />
    </label>
    <label class="field">
      <span>Key name</span>
      <input
        aria-label="Key name"
        value={profile.key_name ?? ""}
        placeholder="none (e.g. a local model)"
        oninput={(e) => set(name, "key_name", e.currentTarget.value.trim())}
      />
    </label>
    <div class="row">
      <label class="field">
        <span>$ per 1M tokens in</span>
        <input
          type="number"
          step="0.01"
          min="0"
          value={profile.price_in ?? ""}
          oninput={(e) => set(name, "price_in", price(e.currentTarget.value))}
        />
      </label>
      <label class="field">
        <span>$ per 1M tokens out</span>
        <input
          type="number"
          step="0.01"
          min="0"
          value={profile.price_out ?? ""}
          oninput={(e) => set(name, "price_out", price(e.currentTarget.value))}
        />
      </label>
    </div>
    <label class="check">
      <input
        type="checkbox"
        checked={profile.tools !== false}
        onchange={(e) => set(name, "tools", e.currentTarget.checked ? undefined : false)}
      />
      Offer tools to this model
    </label>
    <label class="check">
      <input
        type="checkbox"
        checked={profile.vision === true}
        onchange={(e) => set(name, "vision", e.currentTarget.checked ? true : undefined)}
      />
      Can see pictures
    </label>
    <button class="link danger-text" onclick={() => remove(name)}>Remove {name}</button>
  </fieldset>
{/each}

<div class="row add">
  <input aria-label="New profile name" placeholder="New profile name" bind:value={newName} />
  <button onclick={add}>Add profile</button>
</div>

<SectionBar
  {section}
  managed={$store.managed}
  onSave={() => store.save("models")}
  onRevert={() => store.revert("models")}
/>

<h3>API keys</h3>
<p class="hint">
  Kept in the keyring of the machine Lucy runs on. They're never shown again; set one to replace
  it.
</p>
{#if keyProblem !== null}<div class="error" role="alert">{keyProblem}</div>{/if}
{#each $store.keys as key (key.name)}
  <div class="key">
    <div>
      <strong>{key.name}</strong>
      <span class="muted">for {key.used_by.join(", ")} · {where(key.source)}</span>
      {#if key.error}<div class="error">{key.error}</div>{/if}
    </div>
    <div class="row">
      <input
        type="password"
        autocomplete="off"
        aria-label={`New key for ${key.name}`}
        placeholder="Paste a key"
        bind:value={typed[key.name]}
      />
      <button aria-label={`Set key for ${key.name}`} onclick={() => setKey(key.name)}>Set</button>
      {#if key.source === "keyring"}
        <button class="link" onclick={() => store.removeKey(key.name)}>Remove</button>
      {/if}
    </div>
  </div>
{:else}
  <p class="muted">No profile or MCP server needs a key.</p>
{/each}

<style>
  .key {
    padding: 8px 0;
    border-bottom: 1px solid var(--line);
  }
  .add {
    margin: 8px 0;
  }
</style>
