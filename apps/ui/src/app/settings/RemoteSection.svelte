<!-- Remote access: let other machines (and later a phone) use this core over WebSocket. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import SectionBar from "./SectionBar.svelte";

  let { store, onToken }: { store: SettingsStore; onToken?: (rotate: boolean) => Promise<string> } =
    $props();

  const section = $derived($store.sections.remote);
  const remote = $derived(section.draft.remote ?? {});
  let token: string | null = $state(null);
  let problem: string | null = $state(null);

  function set(field: string, value: unknown): void {
    store.edit("remote", (d) => {
      d.remote[field] = value;
    });
  }

  async function showToken(rotate: boolean): Promise<void> {
    if (onToken === undefined) {
      return;
    }
    problem = null;
    try {
      token = await onToken(rotate);
    } catch (err) {
      problem = err instanceof Error ? err.message : String(err);
    }
  }
</script>

<h2>Remote access</h2>
<p class="hint">
  Lets <code>lucy</code> on another machine (or this app in a browser) use this core: the same
  chats, models and keys, with tools running here. It's encrypted with this core's own
  certificate, which the other machine pins by its fingerprint. Changes apply straight away.
</p>

<label class="check">
  <input type="checkbox" checked={remote.enabled === true} onchange={(e) => set("enabled", e.currentTarget.checked)} />
  On
</label>
<label class="field">
  <span>Listen on (address:port)</span>
  <input value={remote.bind ?? "127.0.0.1:7077"} oninput={(e) => set("bind", e.currentTarget.value.trim())} />
</label>
<label class="check">
  <input
    type="checkbox"
    checked={remote.allow_network === true}
    onchange={(e) => set("allow_network", e.currentTarget.checked)}
  />
  Allow addresses beyond this machine (needed for any address other than 127.0.0.1)
</label>
<label class="check">
  <input
    type="checkbox"
    checked={remote.allow_config === true}
    onchange={(e) => set("allow_config", e.currentTarget.checked)}
  />
  Let other machines change settings, keys, the persona, memory and tasks (they can always read
  settings)
</label>
<label class="check">
  <input
    type="checkbox"
    checked={remote.approvals === "remote"}
    onchange={(e) => set("approvals", e.currentTarget.checked ? "remote" : "local")}
  />
  Let other machines approve their own chats' tool calls (otherwise you approve them here; with
  this on, whoever has the token can do what you can, up to the level below)
</label>
<label class="check">
  <input type="checkbox" checked={remote.tls !== false} onchange={(e) => set("tls", e.currentTarget.checked)} />
  Encrypt with this core's own certificate (turn off only behind a proxy that adds TLS)
</label>
<label class="field">
  <span>Highest agent level for their chats</span>
  <select value={String(remote.max_level ?? 1)} onchange={(e) => set("max_level", Number(e.currentTarget.value))}>
    <option value="0">0 · ask for everything</option>
    <option value="1">1 · ask before risky steps</option>
    <option value="2">2 · free inside the workspace</option>
    <option value="3">3 · never ask</option>
  </select>
</label>

<SectionBar
  {section}
  managed={$store.managed}
  onSave={() => store.save("remote")}
  onRevert={() => store.revert("remote")}
/>

{#if onToken !== undefined}
  <h3>Token</h3>
  <p class="hint">
    Other machines need it and the certificate fingerprint to connect
    (<code>lucy remote connect host:7077</code>). A new token disconnects every other machine.
  </p>
  <div class="row">
    <button onclick={() => showToken(false)}>Show token</button>
    <button onclick={() => showToken(true)}>New token</button>
  </div>
  {#if token !== null}<pre>{token}</pre>{/if}
  {#if problem !== null}<div class="error" role="alert">{problem}</div>{/if}
{/if}
