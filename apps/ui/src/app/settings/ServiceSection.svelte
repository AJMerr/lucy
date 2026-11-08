<!-- Service & startup (desktop app only): run the core from login, restart it, and apply the
     sprite's start-at-login setting. These run `lucy service …` on this machine. -->
<script lang="ts">
  import { onMount } from "svelte";

  let { run }: { run: (args: string[]) => Promise<string> } = $props();

  let status = $state("");
  let output = $state("");
  let busy = $state(false);

  async function refresh(): Promise<void> {
    try {
      status = await run(["service", "status"]);
    } catch (err) {
      status = err instanceof Error ? err.message : String(err);
    }
  }

  async function act(args: string[]): Promise<void> {
    busy = true;
    try {
      output = await run(args);
    } catch (err) {
      output = err instanceof Error ? err.message : String(err);
    }
    busy = false;
    await refresh();
  }

  onMount(refresh);
</script>

<h2>Service &amp; startup</h2>
<p class="hint">
  As a login service, Lucy's core starts when you log in and keeps running, so the sprite, this
  window and other machines can always reach it. Without it, the core starts when something needs
  it and stops after 30 idle minutes. Installing also applies the sprite's “start at login”
  setting.
</p>
<pre class="status">{status || "Checking…"}</pre>
<div class="row">
  <button class="primary" disabled={busy} onclick={() => act(["service", "install"])}>
    Install (start at login)
  </button>
  <button disabled={busy} onclick={() => act(["service", "restart"])}>Restart the core</button>
  <button disabled={busy} onclick={() => act(["service", "uninstall"])}>Uninstall</button>
</div>
{#if output}<pre>{output}</pre>{/if}

<style>
  .status {
    margin: 10px 0;
  }
</style>
