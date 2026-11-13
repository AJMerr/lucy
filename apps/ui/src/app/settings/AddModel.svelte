<!-- Add a model without knowing its ID, like `lucy config add-model`: pick a provider, give its
     key if it needs one, choose from the models it actually offers, try the one you picked, and
     save it as a profile (with its context size and sight, when the provider says). -->
<script lang="ts">
  import { onMount } from "svelte";

  import type { ProviderPreset } from "../../lib/setup.ts";
  import type { ConfigJson, SettingsStore } from "../../lib/stores/settings.ts";

  let { store, onDone }: { store: SettingsStore; onDone: (name: string) => void } = $props();
  const core = store.core;

  /** What a provider says about one of its models (`lucy_providers::models::ModelInfo`). */
  interface ModelInfo {
    id: string;
    name: string | null;
    context_tokens: number | null;
    vision: boolean | null;
  }

  let presets: ProviderPreset[] = $state([]);
  let chosen = $state(-1);
  let baseUrl = $state("");
  let keyName = $state("");
  let key = $state("");
  // The key typed for this provider, sent with `models.list` and `models.try` (it was also saved).
  let typedKey = $state("");
  /** Null until the provider has been asked. */
  let models = $state<ModelInfo[] | null>(null);
  let search = $state("");
  let model = $state("");
  let profileName = $state("");
  let tried = $state<{ reply: string; info: ModelInfo | null } | null>(null);
  let tryProblem: string | null = $state(null);
  let problem: string | null = $state(null);
  let busy = $state(false);

  const preset = $derived(presets[chosen]);
  const custom = $derived(preset !== undefined && preset.model === null);
  const hasAddress = $derived(preset !== undefined && preset.provider !== "anthropic");
  const shown = $derived(
    (models ?? []).filter((m) => m.id.toLowerCase().includes(search.trim().toLowerCase())),
  );
  const taken = $derived(profileName.trim() !== "" && $store.config?.profiles?.[profileName.trim()] !== undefined);
  const canSave = $derived(model.trim() !== "" && profileName.trim() !== "" && !taken && !busy);

  onMount(async () => {
    try {
      presets = await core.call<ProviderPreset[]>("setup.presets");
    } catch (err) {
      problem = messageOf(err);
    }
  });

  function pick(index: number): void {
    chosen = index;
    const next = presets[index];
    baseUrl = next?.base_url ?? "";
    keyName = next?.key_name ?? "";
    key = "";
    typedKey = "";
    models = null;
    search = "";
    choose("");
  }

  function choose(id: string): void {
    model = id;
    profileName = id === "" ? "" : nameFor(id);
    tried = null;
    tryProblem = null;
  }

  /** Where the provider is and which key it takes, as the core's methods want them. */
  function access(): Record<string, string> {
    const out: Record<string, string> = { provider: preset?.provider ?? "openai_compat" };
    if (hasAddress && baseUrl.trim() !== "") {
      out.base_url = baseUrl.trim();
    }
    if (keyName.trim() !== "") {
      out.key_name = keyName.trim();
    }
    return out;
  }

  /** `access()` for asking the provider: a key typed here goes along, since a stored one is
   * only sent where it's already used. Never part of the saved profile (it lives in the keyring). */
  function asking(): Record<string, string> {
    return typedKey === "" ? access() : { ...access(), key: typedKey };
  }

  async function find(): Promise<void> {
    busy = true;
    problem = null;
    try {
      if (keyName.trim() !== "" && key.trim() !== "") {
        await store.setKey(keyName.trim(), key.trim());
        typedKey = key.trim();
        key = "";
      }
      models = await core.call<ModelInfo[]>("models.list", asking());
    } catch (err) {
      models = [];
      problem = `Couldn't list its models (${messageOf(err)}). Type the model's ID instead.`;
    }
    busy = false;
  }

  async function tryIt(): Promise<void> {
    busy = true;
    tried = null;
    tryProblem = null;
    try {
      tried = await core.call("models.try", { ...asking(), model: model.trim() });
    } catch (err) {
      tryProblem = messageOf(err);
    }
    busy = false;
  }

  async function save(): Promise<void> {
    const id = model.trim();
    const known = models?.find((m) => m.id === id);
    const vision = tried?.info?.vision ?? known?.vision ?? false;
    const context = tried?.info?.context_tokens ?? known?.context_tokens ?? null;
    const profile: ConfigJson = { ...access(), model: id };
    if (vision) {
      profile.vision = true;
    }
    if (context !== null) {
      profile.context_tokens = context;
    }
    busy = true;
    problem = null;
    try {
      await store.addProfile(profileName.trim(), profile);
      onDone(profileName.trim());
    } catch (err) {
      problem = messageOf(err);
    }
    busy = false;
  }

  /** A profile name from a model ID: "qwen2.5vl:7b" → "qwen2-5vl-7b". */
  function nameFor(id: string): string {
    const last = id.split("/").pop() ?? id;
    return last
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
  }

  function label(m: ModelInfo): string {
    let text = m.id;
    if (m.name) {
      text += ` (${m.name})`;
    }
    if (m.vision === true) {
      text += " · sees pictures";
    }
    return text;
  }

  const messageOf = (err: unknown) => (err instanceof Error ? err.message : String(err));
</script>

<section class="add-model" aria-label="Add a model">
  <h3>Add a model</h3>
  <div class="presets" role="radiogroup" aria-label="Provider">
    {#each presets as option, i (option.profile_name)}
      <label class="preset" class:chosen={i === chosen}>
        <input type="radio" name="add-model-provider" checked={i === chosen} onchange={() => pick(i)} />
        {option.label}
      </label>
    {/each}
  </div>

  {#if preset !== undefined}
    {#if hasAddress}
      <label class="field">
        <span>Address</span>
        <input aria-label="Address" placeholder="http://localhost:8080/v1" bind:value={baseUrl} />
      </label>
    {/if}
    {#if custom}
      <label class="field">
        <span>Key name (leave empty if it needs none)</span>
        <input aria-label="Key name" bind:value={keyName} />
      </label>
    {/if}
    {#if keyName.trim() !== ""}
      <label class="field">
        <span>API key</span>
        <input
          type="password"
          autocomplete="off"
          aria-label="API key"
          placeholder="leave empty if it's already saved"
          bind:value={key}
        />
      </label>
    {/if}
    <button disabled={busy} onclick={find}>Find models</button>
  {/if}

  {#if problem !== null}<div class="error" role="alert">{problem}</div>{/if}

  {#if models !== null}
    {#if models.length > 0}
      <label class="field">
        <span>Search models</span>
        <input aria-label="Search models" placeholder="part of a name" bind:value={search} />
      </label>
      <label class="field">
        <span>{shown.length} of {models.length} models</span>
        <select aria-label="Model" size="8" value={model} onchange={(e) => choose(e.currentTarget.value)}>
          {#each shown as m (m.id)}<option value={m.id}>{label(m)}</option>{/each}
        </select>
      </label>
    {:else}
      <label class="field">
        <span>Model</span>
        <input aria-label="Model" placeholder="its ID, e.g. qwen3:14b" value={model} oninput={(e) => choose(e.currentTarget.value)} />
      </label>
    {/if}
  {/if}

  {#if model.trim() !== ""}
    <label class="field">
      <span>Profile name</span>
      <input aria-label="Profile name" bind:value={profileName} />
    </label>
    {#if taken}<div class="error">There's already a profile called {profileName.trim()}.</div>{/if}
    <div class="row">
      <button disabled={busy} onclick={tryIt}>Try it</button>
      <button class="primary" aria-label="Save model" disabled={!canSave} onclick={save}>Save</button>
    </div>
    {#if tried !== null}
      <p class="muted">It answered: {tried.reply}</p>
    {:else if tryProblem !== null}
      <p class="error">It didn't answer: {tryProblem}. You can still save it.</p>
    {/if}
  {/if}
</section>

<style>
  .add-model {
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 12px;
    margin: 8px 0;
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 8px;
  }
  .preset {
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 4px 8px;
    cursor: pointer;
  }
  .preset.chosen {
    border-color: var(--accent);
  }
  select {
    width: 100%;
  }
</style>
