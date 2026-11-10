<!-- Tasks: reminders and scheduled tasks she's set up (you make them by asking her), with when
     each runs next; pause, resume, reword, change a task's level or model, or delete. -->
<script lang="ts">
  import type { ScheduleInfo, TasksStore } from "../../lib/stores/tasks.ts";

  let {
    store,
    profiles,
    onClose,
  }: {
    store: TasksStore;
    /** Profile names, for a task's model. */
    profiles: string[];
    onClose: () => void;
  } = $props();

  let editing: string | null = $state(null);
  let draft = $state("");
  let confirming: string | null = $state(null);

  const LEVELS = ["0 · supervised (nothing runs)", "1 · guarded", "2 · workspace", "3 · unrestricted"];

  function next(schedule: ScheduleInfo): string {
    if (schedule.paused) {
      return "paused";
    }
    if (schedule.next_run_ms === null) {
      return "done";
    }
    return `next ${new Date(schedule.next_run_ms).toLocaleString(undefined, {
      weekday: "short",
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    })}`;
  }
</script>

<div class="tasks">
  <header>
    <button class="link" onclick={onClose}>← Chats</button>
    <h1>Tasks</h1>
  </header>
  <p class="hint">
    Ask Lucy to set these up, e.g. "remind me at 5 to call Mom" or "every weekday at 8, summarise
    my email and calendar". Reminders just show their message; tasks run on their own at their
    level, and both land in the Tasks chat with a notification.
  </p>
  {#if $store.loaded && !$store.alwaysOn}
    <div class="banner">
      Lucy's core doesn't start at login, so these only run while it's running. Turn on "Run at
      login" in Settings → Service &amp; startup (or <code>lucy service install</code>).
    </div>
  {/if}
  {#if $store.error !== null}<div class="error" role="alert">{$store.error}</div>{/if}

  {#if $store.loaded && $store.schedules.length === 0}
    <p class="muted">Nothing is scheduled yet.</p>
  {/if}
  {#each $store.schedules as schedule (schedule.id)}
    <fieldset class:paused={schedule.paused}>
      <legend>{schedule.title}</legend>
      <div class="when">
        {schedule.kind === "task" ? "Task" : "Reminder"} · {schedule.when} · {next(schedule)}
      </div>
      {#if editing === schedule.id}
        <textarea aria-label="What" rows="3" bind:value={draft}></textarea>
        <div class="row">
          <button
            class="primary"
            disabled={draft.trim() === ""}
            onclick={() => {
              store.change(schedule.id, { what: draft.trim() });
              editing = null;
            }}>Save</button
          >
          <button onclick={() => (editing = null)}>Cancel</button>
        </div>
      {:else}
        <p class="what">{schedule.what}</p>
      {/if}
      {#if schedule.kind === "task"}
        <div class="row">
          <label>
            Level
            <select
              aria-label="Level"
              value={String(schedule.level)}
              onchange={(e) => store.change(schedule.id, { level: Number(e.currentTarget.value) })}
            >
              {#each LEVELS as label, level (level)}
                <option value={String(level)}>{label}</option>
              {/each}
            </select>
          </label>
          <label>
            Model
            <select
              aria-label="Model"
              value={schedule.profile}
              onchange={(e) => store.change(schedule.id, { profile: e.currentTarget.value })}
            >
              {#each profiles.includes(schedule.profile) ? profiles : [schedule.profile, ...profiles] as name (name)}
                <option value={name}>{name}</option>
              {/each}
            </select>
          </label>
        </div>
      {/if}
      {#if confirming === schedule.id}
        <div class="row confirm">
          Delete “{schedule.title}”?
          <button
            class="danger"
            onclick={() => {
              store.remove(schedule.id);
              confirming = null;
            }}>Yes, delete</button
          >
          <button onclick={() => (confirming = null)}>Keep</button>
        </div>
      {:else}
        <div class="row">
          <button onclick={() => store.change(schedule.id, { paused: !schedule.paused })}>
            {schedule.paused ? "Resume" : "Pause"}
          </button>
          <button
            onclick={() => {
              editing = schedule.id;
              draft = schedule.what;
            }}>Edit</button
          >
          <button class="link danger-text" onclick={() => (confirming = schedule.id)}>Delete</button>
        </div>
      {/if}
    </fieldset>
  {/each}
</div>

<style>
  .tasks {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    max-width: 760px;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  h1 {
    font-size: 20px;
    margin: 0 0 8px;
  }
  fieldset {
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 10px 14px;
    margin: 0 0 12px;
  }
  fieldset.paused {
    opacity: 0.65;
  }
  legend {
    font-weight: 600;
    padding: 0 4px;
  }
  .when {
    color: var(--muted);
    font-size: 13px;
  }
  .what {
    white-space: pre-wrap;
    margin: 6px 0;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    font: inherit;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    margin-top: 6px;
  }
  label {
    display: flex;
    gap: 6px;
    align-items: center;
    font-size: 13px;
  }
</style>
