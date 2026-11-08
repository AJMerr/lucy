<!-- Sprite: whether she shows, her look (pack, size), how long replies stay up, her hotkey, and
     whether Lucy's app starts at login. -->
<script lang="ts">
  import type { SettingsStore } from "../../lib/stores/settings.ts";
  import SectionBar from "./SectionBar.svelte";

  let { store }: { store: SettingsStore } = $props();

  const section = $derived($store.sections.sprite);
  const sprite = $derived(section.draft.sprite ?? {});
  // `[app] autostart`, or the older `[sprite] autostart` (v0.4.0).
  const atLogin = $derived(section.draft.app?.autostart === true || sprite.autostart === true);

  function setAtLogin(on: boolean): void {
    store.edit("sprite", (d) => {
      d.app ??= {};
      d.app.autostart = on;
      // Said under [app] from now on.
      if (d.sprite.autostart !== undefined) {
        d.sprite.autostart = false;
      }
    });
  }

  function set(field: string, value: unknown): void {
    store.edit("sprite", (d) => {
      if (value === "" || (typeof value === "number" && Number.isNaN(value))) {
        delete d.sprite[field];
      } else {
        d.sprite[field] = value;
      }
    });
  }
</script>

<h2>Sprite</h2>
<p class="hint">Changes show the next time Lucy's app starts (`lucy sprite quit`, then `lucy app`).</p>

<label class="check">
  <input
    type="checkbox"
    aria-label="Show the sprite when Lucy starts"
    checked={sprite.enabled !== false}
    onchange={(e) => set("enabled", e.currentTarget.checked)}
  />
  Show the sprite when Lucy starts (off, `lucy sprite` still shows her)
</label>

<label class="field">
  <span>Sprite pack (a folder in packs/ in your data folder; "placeholder" is built in)</span>
  <input value={sprite.pack ?? ""} oninput={(e) => set("pack", e.currentTarget.value.trim())} />
</label>
<label class="field">
  <span>Size</span>
  <input
    type="number"
    min="0.25"
    max="8"
    step="0.25"
    value={sprite.scale ?? 1}
    oninput={(e) => set("scale", e.currentTarget.valueAsNumber)}
  />
</label>
<label class="field">
  <span>Seconds a finished reply stays up (0: until your next message)</span>
  <input
    type="number"
    min="0"
    value={sprite.bubble_timeout_secs ?? 0}
    oninput={(e) => set("bubble_timeout_secs", e.currentTarget.valueAsNumber)}
  />
</label>
<label class="field">
  <span>Hotkey to talk to her (X11, macOS, Windows; empty turns it off)</span>
  <input value={sprite.hotkey ?? ""} oninput={(e) => set("hotkey", e.currentTarget.value)} />
</label>
<p class="hint">
  On Wayland, bind <code>lucy sprite ask</code> in your compositor instead (Hyprland:
  <code>bind = CTRL SHIFT, SPACE, exec, lucy sprite ask</code>).
</p>
<label class="check">
  <input
    type="checkbox"
    aria-label="Start Lucy at login"
    checked={atLogin}
    onchange={(e) => setAtLogin(e.currentTarget.checked)}
  />
  Start Lucy at login, in the background (applied when the login service is installed, under
  Service &amp; startup)
</label>

<SectionBar
  {section}
  managed={$store.managed}
  onSave={() => store.save("sprite")}
  onRevert={() => store.revert("sprite")}
/>
