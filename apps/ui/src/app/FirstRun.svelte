<!-- First-run setup, like `lucy config init`: pick a provider (the core's presets), check the
     model, paste a key if it needs one, say your name, and start. -->
<script lang="ts">
  import { onMount } from "svelte";

  import type { Transport } from "../lib/rpc/transport.ts";
  import { type ProviderPreset, completeSetup } from "../lib/setup.ts";

  let { core, onDone }: { core: Transport; onDone: () => void } = $props();

  let presets: ProviderPreset[] = $state([]);
  let chosen = $state(0);
  let model = $state("");
  let profileName = $state("");
  let baseUrl = $state("");
  let keyName = $state("");
  let key = $state("");
  let userName = $state("");
  let problem: string | null = $state(null);
  let busy = $state(false);

  const preset = $derived(presets[chosen]);
  const custom = $derived(preset !== undefined && preset.model === null);
  const needsKey = $derived(preset !== undefined && (custom ? keyName.trim() !== "" : preset.key_name !== null));

  onMount(async () => {
    try {
      presets = await core.call<ProviderPreset[]>("setup.presets");
      pick(0);
    } catch (err) {
      problem = err instanceof Error ? err.message : String(err);
    }
  });

  function pick(index: number): void {
    chosen = index;
    const next = presets[index];
    model = next?.model ?? "";
    profileName = next?.profile_name ?? "";
    baseUrl = "";
    keyName = "";
  }

  async function start(): Promise<void> {
    if (preset === undefined) {
      return;
    }
    busy = true;
    problem = null;
    try {
      await completeSetup(core, { preset, profileName, model, baseUrl, keyName, key, userName });
      key = "";
      onDone();
    } catch (err) {
      problem = err instanceof Error ? err.message : String(err);
    }
    busy = false;
  }
</script>

<main>
  <h1>Hi. Let's set Lucy up.</h1>
  <p class="hint">Which model should she use? You can add more, and change all of this, in Settings.</p>

  <div class="presets" role="radiogroup" aria-label="Provider">
    {#each presets as option, i (option.profile_name)}
      <label class="preset" class:chosen={i === chosen}>
        <input type="radio" name="preset" checked={i === chosen} onchange={() => pick(i)} />
        {option.label}
      </label>
    {/each}
  </div>

  {#if preset !== undefined}
    {#if custom}
      <label class="field">
        <span>Base URL</span>
        <input aria-label="Base URL" placeholder="http://localhost:8080/v1" bind:value={baseUrl} />
      </label>
    {/if}
    <label class="field">
      <span>Model</span>
      <input aria-label="Model" bind:value={model} />
    </label>
    {#if custom}
      <label class="field">
        <span>Key name (if it needs a key)</span>
        <input aria-label="Key name (if it needs a key)" placeholder="e.g. my-server" bind:value={keyName} />
      </label>
    {/if}
    {#if needsKey}
      <label class="field">
        <span>API key</span>
        <input aria-label="API key" type="password" autocomplete="off" bind:value={key} />
      </label>
      <p class="hint">It goes to your keyring and isn't shown again. You can also add it later.</p>
    {/if}
    <label class="field">
      <span>Profile name</span>
      <input bind:value={profileName} />
    </label>
    <label class="field">
      <span>What should Lucy call you?</span>
      <input aria-label="What should Lucy call you?" placeholder="optional" bind:value={userName} />
    </label>
  {/if}

  {#if problem !== null}<div class="error" role="alert">{problem}</div>{/if}
  <button class="primary start" disabled={busy || preset === undefined} onclick={start}>Start</button>
</main>

<style>
  main {
    max-width: 40em;
    margin: 0 auto;
    padding: 32px 20px;
  }
  h1 {
    color: var(--accent);
    font-weight: 600;
  }
  .presets {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 16px 0;
  }
  .preset {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 10px 12px;
    border: 1px solid var(--line);
    border-radius: 8px;
    cursor: pointer;
  }
  .preset.chosen {
    border-color: var(--accent);
    background: var(--selected);
  }
  .start {
    margin-top: 16px;
    padding: 8px 20px;
  }
</style>
