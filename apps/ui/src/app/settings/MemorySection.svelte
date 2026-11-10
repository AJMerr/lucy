<!-- Memory: what Lucy remembers about you, fact by fact (edit or forget each) or as the whole
     file, and whether she remembers at all. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import SectionBar from "./SectionBar.svelte";

  let { store }: { store: SettingsStore } = $props();

  const section = $derived($store.sections.memory);
  const settings = $derived(section.draft.memory ?? {});
  const memory = $derived($store.memory);
  // biome-ignore lint: profiles are the core's shape.
  const profiles: [string, any][] = $derived(Object.entries($store.config?.profiles ?? {}));
  /** Embedding models come through the OpenAI-compatible API (Ollama, OpenAI, OpenRouter). */
  const embedders = $derived(profiles.filter(([, p]) => p.provider === "openai_compat"));
  const headings = $derived(
    (memory?.facts ?? []).reduce<{ heading: string; facts: string[] }[]>((groups, fact) => {
      const group = groups.find((g) => g.heading === fact.heading);
      if (group === undefined) {
        groups.push({ heading: fact.heading, facts: [fact.text] });
      } else {
        group.facts.push(fact.text);
      }
      return groups;
    }, []),
  );

  let editing: string | null = $state(null);
  let draftFact = $state("");
  let asText = $state(false);
  let draftText = $state("");
  let problem: string | null = $state(null);
  let busy = $state(false);

  async function save(text: string): Promise<void> {
    busy = true;
    problem = null;
    try {
      await store.saveMemory(text);
      editing = null;
      asText = false;
    } catch (err) {
      problem = err instanceof Error ? err.message : String(err);
    }
    busy = false;
  }

  /** memory.md with the line for `fact` replaced (or removed, for `null`), the rest as it was. */
  function withFact(fact: string, replacement: string | null): string {
    const lines = (memory?.text ?? "").split("\n");
    const at = lines.findIndex((line) => /^\s*[-*] /.test(line) && line.trim().slice(2).trim() === fact);
    if (at === -1) {
      return memory?.text ?? "";
    }
    if (replacement === null) {
      lines.splice(at, 1);
    } else {
      lines[at] = `- ${replacement.trim()}`;
    }
    return lines.join("\n");
  }

  function set(field: string, value: unknown): void {
    store.edit("memory", (d) => {
      d.memory ??= {};
      if (value === undefined || value === "") {
        delete d.memory[field];
      } else {
        d.memory[field] = value;
      }
    });
  }
</script>

<h2>Memory</h2>
<p class="hint">
  Lucy saves facts about you as you chat (she shows a short note when she does, and asks first for
  anything sensitive; secrets are never saved). Tell her "forget that" in any chat, or change them
  here. Incognito chats don't read or save memories.
</p>

<label class="check">
  <input
    type="checkbox"
    aria-label="Let Lucy remember things about you"
    checked={settings.enabled !== false}
    onchange={(e) => set("enabled", e.currentTarget.checked ? undefined : false)}
  />
  Let Lucy remember things about you
</label>
<label class="field">
  <span>Find memories by meaning with (optional; otherwise by keyword)</span>
  <select
    aria-label="Find memories by meaning with"
    value={settings.embedding_profile ?? ""}
    onchange={(e) => set("embedding_profile", e.currentTarget.value)}
  >
    <option value="">Keywords (no extra model)</option>
    {#each embedders as [name, profile] (name)}
      <option value={name}>{name} ({profile.model})</option>
    {/each}
  </select>
</label>
<p class="hint">
  An embedding model, e.g. Ollama's <code>nomic-embed-text</code> (add it as a profile under Models
  &amp; keys), finds "my pet" when she saved "Has a dog". Only matters once she remembers a lot.
</p>
<SectionBar
  {section}
  managed={$store.managed}
  onSave={() => store.save("memory")}
  onRevert={() => store.revert("memory")}
/>

<h3>What she remembers</h3>
{#if memory === null}
  <p class="muted">Memory can't be loaded until Lucy is set up.</p>
{:else if asText}
  <textarea aria-label="Memory" rows="18" bind:value={draftText}></textarea>
  <div class="row buttons">
    <button class="primary" disabled={busy || draftText === memory.text} onclick={() => save(draftText)}>
      Save memory
    </button>
    <button disabled={busy} onclick={() => (asText = false)}>Cancel</button>
  </div>
{:else}
  {#if headings.length === 0}
    <p class="muted">Nothing yet. She'll save facts as you chat.</p>
  {/if}
  {#each headings as group (group.heading)}
    <fieldset>
      <legend>{group.heading}</legend>
      <ul>
        {#each group.facts as fact (fact)}
          <li>
            {#if editing === fact}
              <input aria-label="Fact" bind:value={draftFact} />
              <button
                class="primary"
                aria-label="Save fact"
                disabled={busy || draftFact.trim() === ""}
                onclick={() => save(withFact(fact, draftFact))}>Save</button
              >
              <button disabled={busy} onclick={() => (editing = null)}>Cancel</button>
            {:else}
              <span>{fact}</span>
              <button
                class="link"
                aria-label={`Edit “${fact}”`}
                onclick={() => {
                  editing = fact;
                  draftFact = fact;
                }}>Edit</button
              >
              <button
                class="link danger-text"
                aria-label={`Forget “${fact}”`}
                disabled={busy}
                onclick={() => save(withFact(fact, null))}>Forget</button
              >
            {/if}
          </li>
        {/each}
      </ul>
    </fieldset>
  {/each}
  <div class="row buttons">
    <button
      onclick={() => {
        draftText = memory.text;
        asText = true;
      }}>Edit as text</button
    >
  </div>
  <p class="muted">Saved in {memory.path}</p>
{/if}
{#if problem !== null}<div class="error" role="alert">{problem}</div>{/if}

<style>
  ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
  }
  li span {
    flex: 1;
  }
  li input {
    flex: 1;
  }
</style>
