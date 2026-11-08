// Sprite: whether she shows, her look, and whether the app starts at login.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import SpriteSection from "./SpriteSection.svelte";

afterEach(cleanup);

async function setUp(sprite: Record<string, unknown>) {
  const core = new FakeCore();
  core.answers["config.get"] = {
    path: "/c.toml",
    managed: false,
    config: { defaults: {}, profiles: {}, sprite, app: { autostart: false } },
  };
  core.answers["keys.status"] = [];
  core.answers["persona.get"] = { name: "lucy", text: "", overridden: false, path: null };
  core.answers["config.update"] = { restart_needed: false };
  const store = new SettingsStore(core);
  await store.load();
  render(SpriteSection, { store });
  return core;
}

test("she can be turned off, and the app set to start at login", async () => {
  const core = await setUp({ enabled: true, pack: "placeholder", autostart: false });

  await fireEvent.click(screen.getByLabelText("Show the sprite when Lucy starts"));
  await fireEvent.click(screen.getByLabelText("Start Lucy at login"));
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    { patch: { sprite: { enabled: false }, app: { autostart: true } } },
  ]);
});

test("the older [sprite] autostart shows as on, and changing it moves it to [app]", async () => {
  const core = await setUp({ enabled: true, pack: "placeholder", autostart: true });
  const box = screen.getByLabelText("Start Lucy at login") as HTMLInputElement;
  assert.equal(box.checked, true);

  await fireEvent.click(box);
  await fireEvent.click(box);
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    { patch: { sprite: { autostart: false }, app: { autostart: true } } },
  ]);
});
