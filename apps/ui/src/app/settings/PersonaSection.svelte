<!-- Persona: edit how Lucy talks (your own copy of the persona file), or go back to the
     built-in one. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";

  let { store }: { store: SettingsStore } = $props();

  const persona = $derived($store.persona);
  let text = $state("");
  let loadedFrom: string | null = $state(null);
  let problem: string | null = $state(null);
  let busy = $state(false);

  // Take the saved text whenever it changes (load, save, reset), not while you type.
  $effect(() => {
    const saved = persona?.text ?? "";
    if (saved !== loadedFrom) {
      loadedFrom = saved;
      text = saved;
    }
  });

  async function run(action: () => Promise<void>): Promise<void> {
    busy = true;
    problem = null;
    try {
      await action();
    } catch (err) {
      problem = err instanceof Error ? err.message : String(err);
    }
    busy = false;
  }
</script>

<h2>Persona</h2>
{#if persona === null}
  <p class="muted">The persona can't be loaded until Lucy is set up.</p>
{:else}
  <p class="hint">
    {#if persona.overridden}
      Lucy uses your version of “{persona.name}”{persona.path ? ` (${persona.path})` : ""}.
    {:else}
      Lucy uses the built-in “{persona.name}”. Saving makes your own copy; reset goes back.
    {/if}
    Text between <code>&lt;!-- voice --&gt;</code> markers is dropped in plain mode, and the rules
    at the end keep her accurate whatever the voice.
  </p>
  <textarea aria-label="Persona" rows="22" bind:value={text}></textarea>
  {#if problem !== null}<div class="error" role="alert">{problem}</div>{/if}
  <div class="row buttons">
    <button class="primary" disabled={busy || text === persona.text} onclick={() => run(() => store.savePersona(text))}>
      Save
    </button>
    <button disabled={busy || text === persona.text} onclick={() => (text = persona.text)}>Revert</button>
    {#if persona.overridden}
      <button class="link" disabled={busy} onclick={() => run(() => store.resetPersona())}>
        Reset to the built-in persona
      </button>
    {/if}
  </div>
{/if}

<style>
  textarea {
    width: 100%;
    box-sizing: border-box;
    font: 13px/1.5 ui-monospace, monospace;
    resize: vertical;
  }
  .buttons {
    margin-top: 10px;
  }
</style>
