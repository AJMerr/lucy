// First-run setup in the app: no config yet, so pick a provider and go.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { FakeCore } from "../lib/testing.ts";
import FirstRun from "./FirstRun.svelte";

afterEach(cleanup);

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function core(): FakeCore {
  const fake = new FakeCore();
  fake.answers["setup.presets"] = [
    {
      label: "Local Qwen via Ollama",
      profile_name: "local-qwen",
      provider: "openai_compat",
      base_url: "http://localhost:11434/v1",
      model: "qwen3:14b",
      key_name: null,
    },
    {
      label: "Anthropic (Claude)",
      profile_name: "claude",
      provider: "anthropic",
      base_url: null,
      model: "claude-sonnet-5-5",
      key_name: "anthropic",
    },
    {
      label: "Other OpenAI-compatible endpoint",
      profile_name: "custom",
      provider: "openai_compat",
      base_url: null,
      model: null,
      key_name: null,
    },
  ];
  fake.answers["config.update"] = { restart_needed: false };
  fake.answers["keys.set"] = null;
  return fake;
}

test("picking a hosted provider asks for its key, and Finish sets everything up", async () => {
  const fake = core();
  let done = 0;
  render(FirstRun, { core: fake, onDone: () => done++ });
  await settle();

  await fireEvent.click(screen.getByLabelText("Anthropic (Claude)"));
  assert.equal((screen.getByLabelText("Model") as HTMLInputElement).value, "claude-sonnet-5-5");
  await fireEvent.input(screen.getByLabelText("API key"), { target: { value: "sk-ant-1" } });
  await fireEvent.input(screen.getByLabelText("What should Lucy call you?"), { target: { value: "Sam" } });
  await fireEvent.click(screen.getByRole("button", { name: "Start" }));
  await settle();

  assert.deepEqual(fake.sent("config.update"), [
    {
      patch: {
        defaults: { profile: "claude" },
        profiles: { claude: { provider: "anthropic", model: "claude-sonnet-5-5", key_name: "anthropic" } },
        user: { name: "Sam" },
      },
    },
  ]);
  assert.deepEqual(fake.sent("keys.set"), [{ name: "anthropic", value: "sk-ant-1" }]);
  assert.equal(done, 1);
});

test("a local model needs no key, and a custom endpoint asks where it is", async () => {
  render(FirstRun, { core: core(), onDone: () => {} });
  await settle();

  assert.equal(screen.queryByLabelText("API key"), null, "the first preset is local and keyless");
  await fireEvent.click(screen.getByLabelText("Other OpenAI-compatible endpoint"));

  assert.ok(screen.getByLabelText("Base URL"));
  assert.ok(screen.getByLabelText("Key name (if it needs a key)"));
});

test("a refused setup says why and stays put", async () => {
  const fake = core();
  fake.answers["config.update"] = () => {
    throw new Error("invalid config: profile `custom` needs a base_url");
  };
  let done = 0;
  render(FirstRun, { core: fake, onDone: () => done++ });
  await settle();

  await fireEvent.click(screen.getByLabelText("Other OpenAI-compatible endpoint"));
  await fireEvent.click(screen.getByRole("button", { name: "Start" }));
  await settle();

  assert.ok(screen.getByText(/needs a base_url/));
  assert.equal(done, 0);
});
