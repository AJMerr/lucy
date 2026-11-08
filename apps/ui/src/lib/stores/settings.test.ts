// The settings page's sections: what they show (from config.get), what Save sends (only what
// changed, as a merge patch), and what happens when the core refuses.

import assert from "node:assert/strict";
import { test } from "vitest";

import { FakeCore } from "../testing.ts";
import { SettingsStore, type SettingsState, mergePatch } from "./settings.ts";

const CONFIG = {
  defaults: { profile: "local", persona: "lucy", mode: "assist", agent_level: 1 },
  profiles: {
    local: { provider: "openai_compat", base_url: "http://localhost:11434/v1", model: "qwen3:14b" },
    claude: { provider: "anthropic", model: "claude-opus-5-5", key_name: "anthropic" },
  },
  agent: { max_steps: 25, max_cost_usd: 1, max_tokens: 200000 },
  tools: { builtin: true, max_rounds: 8, private_paths: [], rules: {} },
  user: {},
  remote: { enabled: false, bind: "127.0.0.1:7077", allow_network: false, allow_config: false, max_level: 1 },
  sprite: { pack: "placeholder", scale: 1, bubble_timeout_secs: 0, hotkey: "Ctrl+Shift+Space", autostart: false },
};

function core(): FakeCore {
  const fake = new FakeCore();
  fake.answers["config.get"] = { path: "/home/sam/.config/lucy/config.toml", managed: false, config: structuredClone(CONFIG) };
  fake.answers["keys.status"] = [{ name: "anthropic", used_by: ["claude"], source: null }];
  fake.answers["persona.get"] = { name: "lucy", text: "# Persona: Lucy\n", overridden: false, path: "/p/lucy.md" };
  fake.answers["config.update"] = { restart_needed: false };
  return fake;
}

function watch(store: SettingsStore): () => SettingsState {
  let latest: SettingsState | undefined;
  store.subscribe((state) => {
    latest = state;
  });
  return () => {
    assert.ok(latest);
    return latest;
  };
}

test("a merge patch holds only what changed: new and changed values, and null for removed", () => {
  const before = { a: 1, b: { c: 2, d: [1, 2] }, gone: "x" };
  const after = { a: 1, b: { c: 3, d: [1, 2], e: true } };

  assert.deepEqual(mergePatch(before, after), { b: { c: 3, e: true }, gone: null });
  assert.equal(mergePatch(before, structuredClone(before)), undefined);
});

test("sections show the config as it is, with where it lives", async () => {
  const store = new SettingsStore(core());
  const state = watch(store);

  await store.load();

  assert.equal(state().path, "/home/sam/.config/lucy/config.toml");
  assert.equal(state().sections.models.draft.defaults.profile, "local");
  assert.equal(state().sections.agent.draft.agent.max_steps, 25);
  assert.equal(state().sections.sprite.draft.sprite.pack, "placeholder");
  assert.equal(state().sections.models.dirty, false);
});

test("Save sends only the fields you changed in that section", async () => {
  const fake = core();
  const store = new SettingsStore(fake);
  const state = watch(store);
  await store.load();

  store.edit("agent", (draft) => {
    draft.agent.max_steps = 40;
  });
  assert.equal(state().sections.agent.dirty, true);
  await store.save("agent");

  assert.deepEqual(fake.sent("config.update"), [{ patch: { agent: { max_steps: 40 } } }]);
  assert.equal(state().sections.agent.dirty, false);
});

test("removing a profile sends null for it", async () => {
  const fake = core();
  const store = new SettingsStore(fake);
  watch(store);
  await store.load();

  store.edit("models", (draft) => {
    delete draft.profiles.claude;
  });
  await store.save("models");

  assert.deepEqual(fake.sent("config.update"), [{ patch: { profiles: { claude: null } } }]);
});

test("a change the core refuses stays in the form, with the reason on that section only", async () => {
  const fake = core();
  fake.answers["config.update"] = () => {
    throw new Error("invalid config: default profile `nope` doesn't exist");
  };
  const store = new SettingsStore(fake);
  const state = watch(store);
  await store.load();

  store.edit("models", (draft) => {
    draft.defaults.profile = "nope";
  });
  await store.save("models");

  assert.equal(state().sections.models.error, "invalid config: default profile `nope` doesn't exist");
  assert.equal(state().sections.models.draft.defaults.profile, "nope");
  assert.equal(state().sections.models.dirty, true);
  assert.equal(state().sections.agent.error, null);
});

test("Revert puts the section back as it was", async () => {
  const store = new SettingsStore(core());
  const state = watch(store);
  await store.load();

  store.edit("sprite", (draft) => {
    draft.sprite.scale = 2;
  });
  store.revert("sprite");

  assert.equal(state().sections.sprite.draft.sprite.scale, 1);
  assert.equal(state().sections.sprite.dirty, false);
});

test("a change that needs a core restart says so", async () => {
  const fake = core();
  fake.answers["config.update"] = { restart_needed: true };
  const store = new SettingsStore(fake);
  const state = watch(store);
  await store.load();

  store.edit("remote", (draft) => {
    draft.remote.enabled = true;
  });
  await store.save("remote");

  assert.equal(state().restartNeeded, true);
});

test("a config home-manager owns can be looked at but not saved", async () => {
  const fake = core();
  fake.answers["config.get"] = { path: "/nix/x", managed: true, config: structuredClone(CONFIG) };
  const store = new SettingsStore(fake);
  const state = watch(store);
  await store.load();

  store.edit("agent", (draft) => {
    draft.agent.max_steps = 3;
  });
  await store.save("agent");

  assert.equal(state().managed, true);
  assert.deepEqual(fake.sent("config.update"), []);
  assert.match(state().sections.agent.error ?? "", /home-manager/);
});

test("keys are set and removed through the core, and never kept here", async () => {
  const fake = core();
  fake.answers["keys.set"] = null;
  fake.answers["keys.remove"] = true;
  const store = new SettingsStore(fake);
  const state = watch(store);
  await store.load();
  fake.answers["keys.status"] = [{ name: "anthropic", used_by: ["claude"], source: "keyring" }];

  await store.setKey("anthropic", "sk-secret");
  await store.removeKey("anthropic");

  assert.deepEqual(fake.sent("keys.set"), [{ name: "anthropic", value: "sk-secret" }]);
  assert.deepEqual(fake.sent("keys.remove"), [{ name: "anthropic" }]);
  assert.ok(!JSON.stringify(state()).includes("sk-secret"));
});

test("the persona can be saved and reset", async () => {
  const fake = core();
  fake.answers["persona.set"] = null;
  fake.answers["persona.reset"] = null;
  const store = new SettingsStore(fake);
  const state = watch(store);
  await store.load();

  fake.answers["persona.get"] = { name: "lucy", text: "Be brief.", overridden: true, path: "/p/lucy.md" };
  await store.savePersona("Be brief.");
  assert.equal(state().persona?.overridden, true);

  fake.answers["persona.get"] = { name: "lucy", text: "# Persona: Lucy\n", overridden: false, path: "/p/lucy.md" };
  await store.resetPersona();

  assert.deepEqual(fake.sent("persona.set"), [{ text: "Be brief." }]);
  assert.equal(fake.sent("persona.reset").length, 1);
  assert.equal(state().persona?.overridden, false);
});
