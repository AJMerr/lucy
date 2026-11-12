<!-- Pictures: what makes them (an image model, or ComfyUI on this computer, which can take
     the GPU from Ollama first), where they're saved, and which model describes pictures for
     models that can't see. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import SectionBar from "./SectionBar.svelte";

  let { store }: { store: SettingsStore } = $props();

  const section = $derived($store.sections.images);
  const images = $derived(section.draft.images ?? {});
  const comfyui = $derived(images.comfyui ?? {});
  // biome-ignore lint: profiles are the core's shape.
  const profiles: [string, any][] = $derived(Object.entries($store.config?.profiles ?? {}));
  /** Image models come in through the OpenAI-compatible API (OpenRouter's, for one). */
  const makers = $derived(profiles.filter(([, p]) => p.provider === "openai_compat"));
  const seers = $derived(profiles.filter(([, p]) => p.vision === true));

  /** Set `[images] field` (or `[images.comfyui] field`); empty removes it. */
  function set(field: string, value: string, table: "images" | "comfyui" = "images"): void {
    store.edit("images", (d) => {
      d.images ??= {};
      let target = d.images;
      if (table === "comfyui") {
        d.images.comfyui ??= {};
        target = d.images.comfyui;
      }
      if (value.trim() === "") {
        delete target[field];
      } else {
        target[field] = value.trim();
      }
      if (table === "comfyui" && Object.keys(target).length === 0) {
        delete d.images.comfyui;
      }
    });
  }

  /** `[images.comfyui] unload_ollama`: on unless set to false, so only "off" is written. */
  function setUnload(on: boolean): void {
    store.edit("images", (d) => {
      d.images ??= {};
      d.images.comfyui ??= {};
      if (on) {
        delete d.images.comfyui.unload_ollama;
      } else {
        d.images.comfyui.unload_ollama = false;
      }
      if (Object.keys(d.images.comfyui).length === 0) {
        delete d.images.comfyui;
      }
    });
  }
</script>

<h2>Pictures</h2>

<h3>Making pictures</h3>
<p class="hint">
  Ask her for a picture and she writes the prompt and makes it, or type
  <code>/image</code> and a description to make one straight away. Pictures from a hosted model
  cost money, so she asks first; ComfyUI on this computer is free and just runs.
</p>
<label class="field">
  <span>Make pictures with</span>
  <select
    aria-label="Make pictures with"
    value={images.generator ?? ""}
    onchange={(e) => set("generator", e.currentTarget.value)}
  >
    <option value="">Nothing (off)</option>
    <option value="comfyui">ComfyUI on this computer</option>
    {#each makers as [name, profile] (name)}
      <option value={name}>{name} ({profile.model})</option>
    {/each}
  </select>
</label>
{#if images.generator !== "comfyui"}
  <p class="hint">
    Use a profile for an image model, e.g. OpenRouter's <code>qwen/qwen-image-3-pro</code>
    (add one under Models &amp; keys).
  </p>
{/if}
{#if images.generator === "comfyui"}
  <label class="field">
    <span>ComfyUI address</span>
    <input
      aria-label="ComfyUI address"
      placeholder="http://127.0.0.1:8188"
      value={comfyui.url ?? ""}
      oninput={(e) => set("url", e.currentTarget.value, "comfyui")}
    />
  </label>
  <label class="field">
    <span>Workflow (exported from ComfyUI with "Export (API)"; empty uses Lucy's own)</span>
    <input
      aria-label="Workflow"
      placeholder="~/comfyui/workflows/lucy.json"
      value={comfyui.workflow ?? ""}
      oninput={(e) => set("workflow", e.currentTarget.value, "comfyui")}
    />
  </label>
  <label class="field">
    <span>Checkpoint (a model file in ComfyUI; empty uses its first)</span>
    <input
      aria-label="Checkpoint"
      placeholder="sd_xl_base_1.0.safetensors"
      value={comfyui.checkpoint ?? ""}
      oninput={(e) => set("checkpoint", e.currentTarget.value, "comfyui")}
    />
  </label>
  <label class="check">
    <input
      type="checkbox"
      aria-label="Free the GPU first"
      checked={comfyui.unload_ollama !== false}
      onchange={(e) => setUnload(e.currentTarget.checked)}
    />
    Free the GPU first: unload Ollama's models before each picture (they load again for her
    reply). Turn off if your card holds both.
  </label>
  <p class="hint">
    Workflows can use <code>{"{{prompt}}"}</code>, <code>{"{{negative}}"}</code>,
    <code>{"{{width}}"}</code>, <code>{"{{height}}"}</code>, <code>{"{{seed}}"}</code> and
    <code>{"{{checkpoint}}"}</code>. Setting up ComfyUI: docs/images.md.
  </p>
{/if}
<label class="field">
  <span>Save pictures in</span>
  <input
    aria-label="Save pictures in"
    placeholder="~/Pictures/Lucy"
    value={images.folder ?? ""}
    oninput={(e) => set("folder", e.currentTarget.value)}
  />
</label>

<h3>Seeing pictures</h3>
<p class="hint">
  Models marked "Can see pictures" under Models &amp; keys get the pictures you send. For the
  others, a model that can see describes them.
</p>
<label class="field">
  <span>Describe pictures with</span>
  <select
    aria-label="Describe pictures with"
    value={images.vision_profile ?? ""}
    onchange={(e) => set("vision_profile", e.currentTarget.value)}
  >
    <option value="">Nobody (models that can't see say so)</option>
    {#each seers as [name, profile] (name)}
      <option value={name}>{name} ({profile.model})</option>
    {/each}
  </select>
</label>
{#if seers.length === 0}
  <p class="hint">No model is marked "Can see pictures" yet.</p>
{/if}

<SectionBar
  {section}
  managed={$store.managed}
  onSave={() => store.save("images")}
  onRevert={() => store.revert("images")}
/>
