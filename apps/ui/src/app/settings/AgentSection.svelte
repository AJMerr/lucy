<!-- Chat & agent: how new chats start (mode, agent level, persona), agent mode's caps and
     workspace, the built-in tools and per-tool rules, private paths, and your name. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import SectionBar from "./SectionBar.svelte";

  let { store }: { store: SettingsStore } = $props();

  const section = $derived($store.sections.agent);
  const draft = $derived(section.draft);
  const models = $derived($store.sections.models.draft.defaults ?? {});

  /** Set `table.field`, removing it when empty. */
  function set(table: string, field: string, value: unknown): void {
    store.edit("agent", (d) => {
      d[table] ??= {};
      if (value === "" || value === undefined || (typeof value === "number" && Number.isNaN(value))) {
        delete d[table][field];
      } else {
        d[table][field] = value;
      }
    });
  }

  /** `tool=rule` lines for `tools.rules`. */
  const rulesText = $derived(
    Object.entries(draft.tools?.rules ?? {})
      .map(([tool, rule]) => `${tool}=${rule}`)
      .join("\n"),
  );

  function setRules(text: string): void {
    const rules: Record<string, string> = {};
    for (const line of text.split("\n")) {
      const [tool, rule] = line.split("=").map((s) => s.trim());
      if (tool && rule) {
        rules[tool] = rule;
      }
    }
    set("tools", "rules", rules);
  }
</script>

<h2>Chat &amp; agent</h2>

<h3>You</h3>
<label class="field">
  <span>What Lucy calls you</span>
  <input value={draft.user?.name ?? ""} oninput={(e) => set("user", "name", e.currentTarget.value.trim())} />
</label>

<h3>Agent mode</h3>
<p class="hint">
  In agent mode Lucy works through a task step by step on her own, at the level a chat sets,
  within these caps. Which mode and level new chats start in is under Models &amp; keys.
</p>
<div class="row">
  <label class="field">
    <span>Most steps per task</span>
    <input
      type="number"
      min="1"
      value={draft.agent?.max_steps ?? ""}
      oninput={(e) => set("agent", "max_steps", e.currentTarget.valueAsNumber)}
    />
  </label>
  <label class="field">
    <span>Most cost per task ($)</span>
    <input
      type="number"
      min="0"
      step="0.1"
      value={draft.agent?.max_cost_usd ?? ""}
      oninput={(e) => set("agent", "max_cost_usd", e.currentTarget.valueAsNumber)}
    />
  </label>
  <label class="field">
    <span>Most tokens per task</span>
    <input
      type="number"
      min="1"
      value={draft.agent?.max_tokens ?? ""}
      oninput={(e) => set("agent", "max_tokens", e.currentTarget.valueAsNumber)}
    />
  </label>
</div>
<label class="field">
  <span>Workspace (level 2 runs freely inside it; remote chats work here)</span>
  <input
    value={draft.agent?.workspace ?? ""}
    placeholder="e.g. /home/you/projects"
    oninput={(e) => set("agent", "workspace", e.currentTarget.value.trim())}
  />
</label>

<h3>Tools</h3>
<label class="check">
  <input
    type="checkbox"
    checked={draft.tools?.builtin !== false}
    onchange={(e) => set("tools", "builtin", e.currentTarget.checked)}
  />
  Built-in tools (files, search, web pages, commands)
</label>
<label class="field">
  <span>Most tool rounds per reply</span>
  <input
    type="number"
    min="1"
    value={draft.tools?.max_rounds ?? ""}
    oninput={(e) => set("tools", "max_rounds", e.currentTarget.valueAsNumber)}
  />
</label>
<label class="field">
  <span>Private paths: always ask before reading (one per line)</span>
  <textarea
    rows="3"
    value={(draft.tools?.private_paths ?? []).join("\n")}
    oninput={(e) =>
      set(
        "tools",
        "private_paths",
        e.currentTarget.value
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean),
      )}
  ></textarea>
</label>
<label class="field">
  <span>Rules for particular tools, one per line: tool=allow, ask or deny</span>
  <textarea rows="3" value={rulesText} oninput={(e) => setRules(e.currentTarget.value)}></textarea>
</label>
{#if models.persona && models.persona !== "lucy"}
  <p class="hint">Persona in use: {models.persona}</p>
{/if}

<SectionBar
  {section}
  managed={$store.managed}
  onSave={() => store.save("agent")}
  onRevert={() => store.revert("agent")}
/>
