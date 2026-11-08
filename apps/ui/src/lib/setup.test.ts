// First-run setup in the app, like `lucy config init`: a provider, its model and key, and
// your name become the first config.toml.

import assert from "node:assert/strict";
import { test } from "vitest";

import { type ProviderPreset, completeSetup, setupPatch } from "./setup.ts";
import { FakeCore } from "./testing.ts";

const local: ProviderPreset = {
  label: "Local Qwen via Ollama",
  profile_name: "local-qwen",
  provider: "openai_compat",
  base_url: "http://localhost:11434/v1",
  model: "qwen3:14b",
  key_name: null,
};
const claude: ProviderPreset = {
  label: "Anthropic (Claude)",
  profile_name: "claude",
  provider: "anthropic",
  base_url: null,
  model: "claude-sonnet-5-5",
  key_name: "anthropic",
};
const custom: ProviderPreset = {
  label: "Other OpenAI-compatible endpoint",
  profile_name: "custom",
  provider: "openai_compat",
  base_url: null,
  model: null,
  key_name: null,
};

test("a local model becomes the default profile, with no key", () => {
  const patch = setupPatch({ preset: local, profileName: "local-qwen", model: "qwen3:14b" });

  assert.deepEqual(patch, {
    defaults: { profile: "local-qwen" },
    profiles: {
      "local-qwen": {
        provider: "openai_compat",
        base_url: "http://localhost:11434/v1",
        model: "qwen3:14b",
      },
    },
  });
});

test("a hosted provider keeps its key name, and your name is added when given", () => {
  const patch = setupPatch({
    preset: claude,
    profileName: "claude",
    model: "claude-opus-5-5",
    userName: "  Sam ",
  });

  assert.deepEqual(patch, {
    defaults: { profile: "claude" },
    profiles: { claude: { provider: "anthropic", model: "claude-opus-5-5", key_name: "anthropic" } },
    user: { name: "Sam" },
  });
});

test("a custom endpoint takes the URL and key name you typed", () => {
  const patch = setupPatch({
    preset: custom,
    profileName: "llama",
    model: "llama-3",
    baseUrl: "http://10.0.0.5:8080/v1",
    keyName: "llama-box",
  });

  assert.deepEqual(patch, {
    defaults: { profile: "llama" },
    profiles: {
      llama: {
        provider: "openai_compat",
        base_url: "http://10.0.0.5:8080/v1",
        model: "llama-3",
        key_name: "llama-box",
      },
    },
  });
});

test("finishing writes the config, then stores the key under its name", async () => {
  const core = new FakeCore();
  core.answers["config.update"] = { restart_needed: false };
  core.answers["keys.set"] = null;

  await completeSetup(core, {
    preset: claude,
    profileName: "claude",
    model: "claude-sonnet-5-5",
    key: " sk-ant-123 ",
  });

  assert.deepEqual(
    core.calls.map((c) => c.method),
    ["config.update", "keys.set"],
  );
  assert.deepEqual(core.sent("keys.set"), [{ name: "anthropic", value: "sk-ant-123" }]);
});

test("no key typed means no key stored", async () => {
  const core = new FakeCore();
  core.answers["config.update"] = { restart_needed: false };

  await completeSetup(core, { preset: local, profileName: "local-qwen", model: "qwen3:14b", key: "" });

  assert.deepEqual(
    core.calls.map((c) => c.method),
    ["config.update"],
  );
});
