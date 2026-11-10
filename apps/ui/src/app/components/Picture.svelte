<!-- A picture kept with a chat, fetched from the core when it comes into view. -->
<script lang="ts">
  import type { ImageRef } from "../../lib/protocol.ts";

  let { image, load }: { image: ImageRef; load: (id: string) => Promise<string> } = $props();

  const url = $derived(load(image.id));
</script>

{#await url}
  <span class="waiting">picture…</span>
{:then src}
  <img {src} alt={image.made ? "a picture she made" : "a picture"} />
{:catch}
  <span class="waiting">(the picture is gone)</span>
{/await}

<style>
  img {
    display: block;
    max-width: 100%;
    max-height: var(--picture-height, 240px);
    border: 1px solid var(--line);
    border-radius: 8px;
  }
  .waiting {
    color: var(--muted);
    font-size: 12px;
  }
</style>
