<!-- Save and Revert for one settings section, with why the last save was refused. -->
<script lang="ts">
  import type { Section } from "../../lib/stores/settings.ts";

  let {
    section,
    managed,
    onSave,
    onRevert,
  }: { section: Section; managed: boolean; onSave: () => void; onRevert: () => void } = $props();
</script>

<div class="bar">
  {#if section.error !== null}
    <div class="error" role="alert">{section.error}</div>
  {/if}
  <div class="buttons">
    <button class="primary" disabled={!section.dirty || section.saving || managed} onclick={onSave}>
      {section.saving ? "Saving…" : "Save"}
    </button>
    <button disabled={!section.dirty || section.saving} onclick={onRevert}>Revert</button>
    {#if section.dirty}<span class="note">Unsaved changes</span>{/if}
  </div>
</div>

<style>
  .bar {
    position: sticky;
    bottom: 0;
    padding: 10px 0;
    background: var(--bg);
    border-top: 1px solid var(--line);
    margin-top: 16px;
  }
  .buttons {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .error {
    color: var(--hot);
    margin-bottom: 8px;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
</style>
