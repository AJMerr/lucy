<!-- The open chat's title and its own settings: model (profile), Assist/Agent mode, agent level,
     plain mode, and undo for the last reply's file changes. Like `/model`, `/mode`, `/level`,
     `/plain` and `/undo` in the terminal. -->
<script lang="ts">
  import type { SessionSettings } from "../../lib/stores/chats.ts";

  export interface ProfileInfo {
    name: string;
    provider: string;
    model: string;
    is_default: boolean;
  }

  let {
    title,
    settings,
    profiles,
    busy,
    onProfile,
    onMode,
    onLevel,
    onPlain,
    onUndo,
  }: {
    title: string;
    settings: SessionSettings | null;
    profiles: ProfileInfo[];
    busy: boolean;
    onProfile: (name: string) => void;
    onMode: (mode: SessionSettings["mode"]) => void;
    onLevel: (level: number) => void;
    onPlain: (plain: boolean) => void;
    onUndo: () => void;
  } = $props();

  const levels = [
    { value: 0, label: "0 · ask for everything" },
    { value: 1, label: "1 · ask before risky steps" },
    { value: 2, label: "2 · free inside the workspace" },
    { value: 3, label: "3 · never ask" },
  ];
</script>

<header>
  <h1>{title}</h1>
  {#if settings !== null}
    <div class="controls">
      <label>
        <span>Model</span>
        <select
          aria-label="Model"
          value={settings.profile}
          disabled={busy}
          onchange={(event) => onProfile(event.currentTarget.value)}
        >
          {#each profiles as profile (profile.name)}
            <option value={profile.name}>{profile.name} · {profile.model}</option>
          {/each}
        </select>
      </label>
      <div class="segmented" role="group" aria-label="Mode">
        <button
          aria-pressed={settings.mode === "assist"}
          disabled={busy}
          onclick={() => onMode("assist")}>Assist</button
        >
        <button
          aria-pressed={settings.mode === "agent"}
          disabled={busy}
          onclick={() => onMode("agent")}>Agent</button
        >
      </div>
      {#if settings.mode === "agent"}
        <label>
          <span>Level</span>
          <select
            aria-label="Level"
            value={String(settings.level)}
            disabled={busy}
            onchange={(event) => onLevel(Number(event.currentTarget.value))}
          >
            {#each levels as level (level.value)}
              <option value={String(level.value)}>{level.label}</option>
            {/each}
          </select>
        </label>
      {/if}
      <label class="check">
        <input
          type="checkbox"
          checked={settings.plain}
          disabled={busy}
          onchange={(event) => onPlain(event.currentTarget.checked)}
        />
        Plain
      </label>
      <button class="link" disabled={busy} onclick={onUndo}>Undo changes</button>
    </div>
  {/if}
</header>

<style>
  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 16px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
  }
  h1 {
    font-size: 15px;
    font-weight: 600;
    margin: 0;
    flex: 1;
    min-width: 8em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    font-size: 13px;
  }
  label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
  }
  select {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 3px 6px;
    max-width: 16em;
  }
  .segmented {
    display: flex;
  }
  .segmented button {
    border-radius: 0;
  }
  .segmented button:first-child {
    border-radius: 6px 0 0 6px;
  }
  .segmented button:last-child {
    border-radius: 0 6px 6px 0;
  }
  .segmented button[aria-pressed="true"] {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  .check {
    color: var(--text);
  }
</style>
