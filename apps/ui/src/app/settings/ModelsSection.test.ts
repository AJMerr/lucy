// Models & keys: the profiles (which model, where, with which key), the default one, and the
// API keys they use.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import ModelsSection from "./ModelsSection.svelte";

afterEach(cleanup);

async function setUp() {
  const core = new FakeCore();
  core.answers["config.get"] = {
    path: "/c.toml",
    managed: false,
    config: {
      defaults: { profile: "local", persona: "lucy", mode: "assist", agent_level: 1 },
      profiles: {
        local: { provider: "openai_compat", base_url: "http://localhost:11434/v1", model: "qwen3:14b" },
        claude: { provider: "anthropic", model: "claude-opus-5-5", key_name: "anthropic" },
      },
    },
  };
  core.answers["keys.status"] = [{ name: "anthropic", used_by: ["claude"], source: null }];
  core.answers["persona.get"] = { name: "lucy", text: "", overridden: false, path: null };
  core.answers["config.update"] = { restart_needed: false };
  const store = new SettingsStore(core);
  await store.load();
  render(ModelsSection, { store });
  return core;
}

test("each profile shows its model, and changing one saves just that", async () => {
  const core = await setUp();
  const claude = screen.getByRole("group", { name: "claude" });
  const model = within(claude).getByLabelText("Model") as HTMLInputElement;
  assert.equal(model.value, "claude-opus-5-5");

  await fireEvent.input(model, { target: { value: "claude-sonnet-5-5" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    { patch: { profiles: { claude: { model: "claude-sonnet-5-5" } } } },
  ]);
});

test("a new profile can be added and made the default", async () => {
  const core = await setUp();

  await fireEvent.input(screen.getByLabelText("New profile name"), { target: { value: "work" } });
  await fireEvent.click(screen.getByRole("button", { name: "Add profile" }));
  const work = screen.getByRole("group", { name: "work" });
  await fireEvent.input(within(work).getByLabelText("Model"), { target: { value: "gpt-x" } });
  await fireEvent.input(within(work).getByLabelText("Base URL"), {
    target: { value: "https://api.example.com/v1" },
  });
  await fireEvent.change(screen.getByLabelText("Default profile"), { target: { value: "work" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    {
      patch: {
        defaults: { profile: "work" },
        profiles: {
          work: { provider: "openai_compat", model: "gpt-x", base_url: "https://api.example.com/v1" },
        },
      },
    },
  ]);
});

test("a key is typed into a password box, sent once, and never shown", async () => {
  const core = await setUp();
  core.answers["keys.set"] = null;
  const box = screen.getByLabelText("New key for anthropic") as HTMLInputElement;
  assert.equal(box.type, "password");
  assert.ok(screen.getByText(/missing/));

  await fireEvent.input(box, { target: { value: "sk-ant-secret" } });
  core.answers["keys.status"] = [{ name: "anthropic", used_by: ["claude"], source: "keyring" }];
  await fireEvent.click(screen.getByRole("button", { name: "Set key for anthropic" }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(core.sent("keys.set"), [{ name: "anthropic", value: "sk-ant-secret" }]);
  assert.equal(box.value, "", "the box is cleared");
  assert.ok(screen.getByText(/set, in the keyring/));
  assert.ok(!document.body.innerHTML.includes("sk-ant-secret"));
});

test("a refused save shows why", async () => {
  const core = await setUp();
  core.answers["config.update"] = () => {
    throw new Error("invalid config: profile `claude` needs a model");
  };
  const claude = screen.getByRole("group", { name: "claude" });

  await fireEvent.input(within(claude).getByLabelText("Model"), { target: { value: "" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.ok(screen.getByText("invalid config: profile `claude` needs a model"));
});

test("a model can be marked as able to see pictures", async () => {
  const core = await setUp();
  const claude = screen.getByRole("group", { name: "claude" });

  await fireEvent.click(within(claude).getByLabelText("Can see pictures"));
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [{ patch: { profiles: { claude: { vision: true } } } }]);
});
