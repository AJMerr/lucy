<!-- Voice: talk to Lucy and hear her (in her desktop app). How she hears you, how she sounds,
     a hosted voice's key, reading typed replies aloud and her wake word. Saving gets voice ready,
     downloading what it needs the first time. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import type { Progress, VoiceApp, VoiceStatus } from "../../lib/voice/app.ts";
  import SectionBar from "./SectionBar.svelte";

  let { store, voice }: { store: SettingsStore; voice?: VoiceApp } = $props();

  const section = $derived($store.sections.voice);
  const settings = $derived(section.draft.voice ?? {});
  const engine = $derived(settings.voice ?? "kokoro");

  const HEARING = [
    ["tiny", "tiny (fastest, 116 MB)"],
    ["base", "base (207 MB)"],
    ["small", "small (good on a CPU, 640 MB)"],
    ["turbo", "turbo (most accurate, best with a fast CPU, 560 MB)"],
  ];
  const VOICES = [
    ["kokoro", "Kokoro (on this computer, natural)"],
    ["piper", "Piper (on this computer, light)"],
    ["openai", "OpenAI (hosted, needs a key)"],
    ["elevenlabs", "ElevenLabs (hosted, needs a key)"],
  ];
  const SPEAKERS: Record<string, string[]> = {
    kokoro: ["af_sarah", "af_bella", "af_nicole", "af_sky", "af", "bf_emma", "bf_isabella", "am_adam", "am_michael", "bm_george", "bm_lewis"],
    piper: ["en_US-amy-medium", "en_GB-jenny_dioco-medium"],
    openai: ["nova", "shimmer", "coral", "sage", "alloy", "ash", "ballad", "echo", "fable", "onyx"],
  };
  const speaker = $derived(settings.speaker ?? SPEAKERS[engine]?.[0] ?? "");

  let status: VoiceStatus | null = $state(null);
  let progress: Progress | null = $state(null);
  let preparing = $state(false);
  let key = $state("");
  let keySaved = $state(false);
  let problem: string | null = $state(null);

  $effect(() => voice?.onProgress((p) => (progress = p)));

  function set(field: string, value: unknown): void {
    store.edit("voice", (d) => {
      d.voice ??= {};
      if (value === undefined || value === "" || value === false) {
        delete d.voice[field];
      } else {
        d.voice[field] = value;
      }
    });
  }

  function chooseVoice(value: string): void {
    store.edit("voice", (d) => {
      d.voice ??= {};
      d.voice.voice = value;
      // The new engine's first speaker (ElevenLabs takes an ID you type).
      const first = SPEAKERS[value]?.[0];
      if (first === undefined) {
        delete d.voice.speaker;
      } else {
        d.voice.speaker = first;
      }
    });
  }

  async function prepare(): Promise<void> {
    if (voice === undefined) {
      return;
    }
    preparing = true;
    problem = null;
    try {
      status = await voice.prepare();
    } catch (err) {
      problem = err instanceof Error ? err.message : String(err);
    }
    preparing = false;
    progress = null;
  }

  async function save(): Promise<void> {
    await store.save("voice");
    if ($store.sections.voice.error === null) {
      await prepare();
    }
  }

  async function saveKey(): Promise<void> {
    problem = null;
    try {
      await store.setKey(engine, key.trim());
      key = "";
      keySaved = true;
    } catch (err) {
      problem = err instanceof Error ? err.message : String(err);
    }
  }

  async function preview(): Promise<void> {
    if (status?.ready !== true) {
      await prepare();
    }
    await voice?.say("Hey, it's Lucy. This is how I sound.");
  }
</script>

<h2>Voice</h2>
{#if voice === undefined}
  <p class="hint">
    Voice works in Lucy's desktop app, which has your microphone and speakers. These settings are
    saved here and used there.
  </p>
{:else}
  <p class="hint">
    Hold the mic button (or the key you bind to <code>lucy sprite talk</code>) and talk; she
    answers out loud. Everything she needs is downloaded the first time, and hearing you happens on
    this computer.
  </p>
{/if}

<label class="check">
  <input
    type="checkbox"
    aria-label="Talk to Lucy"
    checked={settings.enabled === true}
    onchange={(e) => set("enabled", e.currentTarget.checked)}
  />
  Talk to Lucy (push-to-talk; replies to what you say are read aloud)
</label>

<label class="field">
  <span>Hearing</span>
  <select aria-label="Hearing" value={settings.hearing ?? "small"} onchange={(e) => set("hearing", e.currentTarget.value)}>
    {#each HEARING as [value, label] (value)}
      <option {value}>{label}</option>
    {/each}
  </select>
</label>

<label class="field">
  <span>Her voice</span>
  <select aria-label="Her voice" value={engine} onchange={(e) => chooseVoice(e.currentTarget.value)}>
    {#each VOICES as [value, label] (value)}
      <option {value}>{label}</option>
    {/each}
  </select>
</label>

<label class="field">
  <span>Which voice</span>
  {#if engine === "elevenlabs"}
    <input
      aria-label="Which voice"
      placeholder="an ElevenLabs voice ID"
      value={settings.speaker ?? ""}
      oninput={(e) => set("speaker", e.currentTarget.value.trim())}
    />
  {:else}
    <select aria-label="Which voice" value={speaker} onchange={(e) => set("speaker", e.currentTarget.value)}>
      {#each SPEAKERS[engine] ?? [] as name (name)}
        <option value={name}>{name}</option>
      {/each}
    </select>
  {/if}
</label>

{#if engine === "openai" || engine === "elevenlabs"}
  <div class="row">
    <input
      type="password"
      aria-label={engine === "openai" ? "OpenAI API key" : "ElevenLabs API key"}
      placeholder={`${engine === "openai" ? "OpenAI" : "ElevenLabs"} API key (stored in the keyring)`}
      bind:value={key}
    />
    <button disabled={key.trim() === ""} onclick={saveKey}>Save key</button>
    {#if keySaved}<span class="muted">Saved.</span>{/if}
  </div>
{/if}

<label class="check">
  <input
    type="checkbox"
    aria-label="Read typed replies aloud"
    checked={settings.read_typed === true}
    onchange={(e) => set("read_typed", e.currentTarget.checked)}
  />
  Read typed replies aloud too
</label>
<label class="check">
  <input
    type="checkbox"
    aria-label="Listen for Hey Lucy"
    checked={settings.wake_word === true}
    onchange={(e) => set("wake_word", e.currentTarget.checked)}
  />
  Listen for "Hey Lucy" (keeps the microphone open while the app runs)
</label>

<SectionBar {section} managed={$store.managed} onSave={save} onRevert={() => store.revert("voice")} />

{#if voice !== undefined}
  <div class="row">
    <button disabled={preparing} onclick={preview}>Preview</button>
    {#if preparing}
      <span class="muted">
        {#if progress !== null}
          Downloading {progress.label}… {Math.floor((progress.done / Math.max(progress.total, 1)) * 100)}%
        {:else}
          Getting voice ready…
        {/if}
      </span>
    {:else if status?.ready}
      <span class="muted">Ready.</span>
    {:else if status?.error}
      <span class="error">{status.error}</span>
    {/if}
  </div>
{/if}
{#if problem !== null}<div class="error" role="alert">{problem}</div>{/if}
