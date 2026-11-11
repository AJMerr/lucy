// Adding a model without knowing its ID: pick a provider, give its key, choose from the models
// it offers, try one, and save it with what the provider says about it.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import AddModel from "./AddModel.svelte";

afterEach(cleanup);

const PRESETS = [
  { label: "Local (Ollama)", profile_name: "local-qwen", provider: "openai_compat", base_url: "http://localhost:11434/v1", model: "qwen3:14b", key_name: null },
  { label: "OpenRouter", profile_name: "openrouter", provider: "openai_compat", base_url: "https://openrouter.ai/api/v1", model: "qwen/qwen3-235b", key_name: "openrouter" },
  { label: "Anthropic", profile_name: "claude", provider: "anthropic", base_url: null, model: "claude-opus-5-5", key_name: "anthropic" },
  { label: "Something else", profile_name: "custom", provider: "openai_compat", base_url: null, model: null, key_name: null },
];

async function setUp() {
  const core = new FakeCore();
  core.answers["config.get"] = {
    path: "/c.toml",
    managed: false,
    config: { defaults: { profile: "local" }, profiles: { local: { provider: "openai_compat", model: "qwen3:14b" } } },
  };
  core.answers["keys.status"] = [];
  core.answers["keys.set"] = {};
  core.answers["persona.get"] = { name: "lucy", text: "", overridden: false, path: null };
  core.answers["config.update"] = { restart_needed: false };
  core.answers["setup.presets"] = PRESETS;
  core.answers["models.list"] = [
    { id: "openai/gpt-5-mini", name: "GPT-5 Mini", context_tokens: 400000, vision: true },
    { id: "qwen/qwen3-235b", name: "Qwen3 235B", context_tokens: 131072, vision: false },
    { id: "z-ai/glm-5", name: "GLM 5", context_tokens: null, vision: null },
  ];
  core.answers["models.try"] = { reply: "Hello!", info: { id: "z-ai/glm-5", name: null, context_tokens: 200000, vision: null } };
  const store = new SettingsStore(core);
  await store.load();
  let done: string | null = null;
  render(AddModel, { store, onDone: (name: string) => (done = name) });
  await screen.findByRole("radio", { name: "OpenRouter" });
  return { core, done: () => done };
}

test("a hosted model is found, tried and saved with its key in the keyring", async () => {
  const { core, done } = await setUp();

  await fireEvent.click(screen.getByRole("radio", { name: "OpenRouter" }));
  await fireEvent.input(screen.getByLabelText("API key"), { target: { value: " sk-or-1 " } });
  await fireEvent.click(screen.getByRole("button", { name: "Find models" }));
  await fireEvent.input(await screen.findByLabelText("Search models"), { target: { value: "glm" } });
  const list = screen.getByLabelText("Model") as HTMLSelectElement;
  assert.deepEqual([...list.options].map((o) => o.value), ["z-ai/glm-5"]);
  await fireEvent.change(list, { target: { value: "z-ai/glm-5" } });
  assert.equal((screen.getByLabelText("Profile name") as HTMLInputElement).value, "glm-5");
  await fireEvent.click(screen.getByRole("button", { name: "Try it" }));
  await screen.findByText(/Hello!/);
  await fireEvent.click(screen.getByRole("button", { name: "Save model" }));

  assert.deepEqual(core.sent("keys.set"), [{ name: "openrouter", value: "sk-or-1" }]);
  assert.deepEqual(core.sent("models.list"), [
    { provider: "openai_compat", base_url: "https://openrouter.ai/api/v1", key_name: "openrouter" },
  ]);
  assert.deepEqual(core.sent("models.try"), [
    { provider: "openai_compat", base_url: "https://openrouter.ai/api/v1", model: "z-ai/glm-5", key_name: "openrouter" },
  ]);
  assert.deepEqual(core.sent("config.update"), [
    {
      patch: {
        profiles: {
          "glm-5": {
            provider: "openai_compat",
            base_url: "https://openrouter.ai/api/v1",
            model: "z-ai/glm-5",
            key_name: "openrouter",
            context_tokens: 200000,
          },
        },
      },
    },
  ]);
  await waitFor(() => assert.equal(done(), "glm-5"));
});

test("sight and context size come from the provider's list when trying says nothing", async () => {
  const { core } = await setUp();
  core.answers["models.try"] = { reply: "Hi", info: null };

  await fireEvent.click(screen.getByRole("radio", { name: "OpenRouter" }));
  await fireEvent.click(screen.getByRole("button", { name: "Find models" }));
  await fireEvent.change(await screen.findByLabelText("Model"), { target: { value: "openai/gpt-5-mini" } });
  await fireEvent.click(screen.getByRole("button", { name: "Try it" }));
  await screen.findByText(/Hi/);
  await fireEvent.click(screen.getByRole("button", { name: "Save model" }));

  const patch = (core.sent("config.update")[0] as { patch: { profiles: Record<string, Record<string, unknown>> } }).patch;
  assert.deepEqual(patch.profiles["gpt-5-mini"], {
    provider: "openai_compat",
    base_url: "https://openrouter.ai/api/v1",
    model: "openai/gpt-5-mini",
    key_name: "openrouter",
    vision: true,
    context_tokens: 400000,
  });
  assert.deepEqual(core.sent("keys.set"), [], "no key typed, none sent");
});

test("when nothing is listed the model can be typed, and a failed try can still be saved", async () => {
  const { core } = await setUp();
  core.answers["models.list"] = () => {
    throw new Error("connection refused");
  };
  core.answers["models.try"] = () => {
    throw new Error("connection refused");
  };

  await fireEvent.click(screen.getByRole("radio", { name: "Something else" }));
  await fireEvent.input(screen.getByLabelText("Address"), { target: { value: "http://box:8080/v1" } });
  await fireEvent.click(screen.getByRole("button", { name: "Find models" }));
  await screen.findByText(/connection refused/);
  await fireEvent.input(screen.getByLabelText("Model"), { target: { value: "my-model" } });
  await fireEvent.click(screen.getByRole("button", { name: "Try it" }));
  await screen.findByText(/didn't answer/);
  await fireEvent.click(screen.getByRole("button", { name: "Save model" }));

  assert.deepEqual(core.sent("config.update"), [
    { patch: { profiles: { "my-model": { provider: "openai_compat", base_url: "http://box:8080/v1", model: "my-model" } } } },
  ]);
});

test("a name that's taken isn't saved over", async () => {
  const { core } = await setUp();
  core.answers["models.list"] = [];

  await fireEvent.click(screen.getByRole("radio", { name: "Local (Ollama)" }));
  await fireEvent.click(screen.getByRole("button", { name: "Find models" }));
  await fireEvent.input(await screen.findByLabelText("Model"), { target: { value: "qwen3:14b" } });
  await fireEvent.input(screen.getByLabelText("Profile name"), { target: { value: "local" } });

  assert.ok((screen.getByRole("button", { name: "Save model" }) as HTMLButtonElement).disabled);
  assert.ok(screen.getByText(/already a profile called local/));
  assert.deepEqual(core.sent("config.update"), []);
});
