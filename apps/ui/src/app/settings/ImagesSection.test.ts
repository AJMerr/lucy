// Pictures: what makes them (an image model, or ComfyUI on this computer), where they're saved,
// and which model describes pictures for models that can't see.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import ImagesSection from "./ImagesSection.svelte";

afterEach(cleanup);

async function setUp(images: Record<string, unknown> = {}) {
  const core = new FakeCore();
  core.answers["config.get"] = {
    path: "/c.toml",
    managed: false,
    config: {
      defaults: { profile: "local" },
      profiles: {
        local: { provider: "openai_compat", base_url: "http://localhost:11434/v1", model: "qwen3:14b" },
        claude: { provider: "anthropic", model: "claude-opus-5-5", key_name: "anthropic", vision: true },
        pics: {
          provider: "openai_compat",
          base_url: "https://openrouter.ai/api/v1",
          model: "google/gemini-2.5-flash-image",
          key_name: "openrouter",
        },
      },
      images,
    },
  };
  core.answers["keys.status"] = [];
  core.answers["persona.get"] = { name: "lucy", text: "", overridden: false, path: null };
  core.answers["config.update"] = { restart_needed: false };
  const store = new SettingsStore(core);
  await store.load();
  render(ImagesSection, { store });
  return core;
}

function options(label: string): string[] {
  const select = screen.getByLabelText(label) as HTMLSelectElement;
  return [...select.options].map((option) => option.value);
}

test("pictures can be made by ComfyUI or a model that makes them, not by Claude", async () => {
  await setUp();

  assert.deepEqual(options("Make pictures with"), ["", "comfyui", "local", "pics"]);
  assert.equal((screen.getByLabelText("Make pictures with") as HTMLSelectElement).value, "");
});

test("choosing ComfyUI shows its settings and saves only what you set", async () => {
  const core = await setUp();
  assert.equal(screen.queryByLabelText("ComfyUI address"), null);

  await fireEvent.change(screen.getByLabelText("Make pictures with"), {
    target: { value: "comfyui" },
  });
  const address = screen.getByLabelText("ComfyUI address") as HTMLInputElement;
  assert.equal(address.placeholder, "http://127.0.0.1:8188");
  await fireEvent.input(screen.getByLabelText("Checkpoint"), {
    target: { value: "juggernautXL.safetensors" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    { patch: { images: { generator: "comfyui", comfyui: { checkpoint: "juggernautXL.safetensors" } } } },
  ]);
});

test("turning picture making off removes the generator, and the folder can be changed", async () => {
  const core = await setUp({ generator: "pics" });

  await fireEvent.change(screen.getByLabelText("Make pictures with"), { target: { value: "" } });
  await fireEvent.input(screen.getByLabelText("Save pictures in"), {
    target: { value: "~/Art/Lucy" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    { patch: { images: { generator: null, folder: "~/Art/Lucy" } } },
  ]);
});

test("only models that can see are offered to describe pictures", async () => {
  const core = await setUp();

  assert.deepEqual(options("Describe pictures with"), ["", "claude"]);
  await fireEvent.change(screen.getByLabelText("Describe pictures with"), {
    target: { value: "claude" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [{ patch: { images: { vision_profile: "claude" } } }]);
});
