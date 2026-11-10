// Memory: what she remembers, fact by fact or as text, and whether she remembers at all.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import MemorySection from "./MemorySection.svelte";

afterEach(cleanup);

const TEXT = "# Lucy's memory\n\n## About you\n- Has a dog called Biscuit\n- Lives in Denver\n\n## Projects\n- Building Lucy\n";

function view(text: string) {
  const facts: { heading: string; text: string }[] = [];
  let heading = "Notes";
  for (const line of text.split("\n")) {
    if (line.startsWith("## ")) heading = line.slice(3);
    else if (line.startsWith("- ")) facts.push({ heading, text: line.slice(2) });
  }
  return { path: "/data/memory.md", text, facts };
}

async function setUp(memory: Record<string, unknown> = {}) {
  const core = new FakeCore();
  core.answers["config.get"] = {
    path: "/c.toml",
    managed: false,
    config: {
      defaults: { profile: "local" },
      profiles: {
        local: { provider: "openai_compat", base_url: "http://localhost:11434/v1", model: "qwen3:14b" },
        embed: { provider: "openai_compat", base_url: "http://localhost:11434/v1", model: "nomic-embed-text" },
        claude: { provider: "anthropic", model: "claude-opus-5-5", key_name: "anthropic" },
      },
      memory,
    },
  };
  core.answers["keys.status"] = [];
  core.answers["persona.get"] = { name: "lucy", text: "", overridden: false, path: null };
  core.answers["memory.get"] = view(TEXT);
  core.answers["memory.set"] = (params: Record<string, unknown>) => view(String(params.text));
  core.answers["config.update"] = { restart_needed: false };
  const store = new SettingsStore(core);
  await store.load();
  render(MemorySection, { store });
  return core;
}

test("facts are listed under their headings", async () => {
  await setUp();

  const about = screen.getByRole("group", { name: "About you" });
  assert.ok(within(about).getByText("Has a dog called Biscuit"));
  assert.ok(within(about).getByText("Lives in Denver"));
  assert.ok(within(screen.getByRole("group", { name: "Projects" })).getByText("Building Lucy"));
});

test("a fact can be deleted, and the rest of the file stays as it was", async () => {
  const core = await setUp();

  await fireEvent.click(screen.getByRole("button", { name: "Forget “Lives in Denver”" }));

  await waitFor(() => assert.equal(core.sent("memory.set").length, 1));
  assert.deepEqual(core.sent("memory.set"), [
    { text: "# Lucy's memory\n\n## About you\n- Has a dog called Biscuit\n\n## Projects\n- Building Lucy\n" },
  ]);
});

test("a fact can be edited in place", async () => {
  const core = await setUp();

  await fireEvent.click(screen.getByRole("button", { name: "Edit “Building Lucy”" }));
  const box = screen.getByRole("textbox", { name: "Fact" });
  await fireEvent.input(box, { target: { value: "Building Lucy 0.7" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save fact" }));

  await waitFor(() => assert.equal(core.sent("memory.set").length, 1));
  assert.ok(String((core.sent("memory.set")[0] as { text: string }).text).includes("- Building Lucy 0.7\n"));
});

test("the whole file can be edited as text", async () => {
  const core = await setUp();

  await fireEvent.click(screen.getByRole("button", { name: "Edit as text" }));
  const box = screen.getByRole("textbox", { name: "Memory" }) as HTMLTextAreaElement;
  assert.equal(box.value, TEXT);
  await fireEvent.input(box, { target: { value: "## People\n- Sister: Maya\n" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save memory" }));

  await waitFor(() => assert.deepEqual(core.sent("memory.set"), [{ text: "## People\n- Sister: Maya\n" }]));
});

test("memory can be switched off, and an embedding model chosen", async () => {
  const core = await setUp();

  const embedding = screen.getByLabelText("Find memories by meaning with") as HTMLSelectElement;
  assert.deepEqual([...embedding.options].map((o) => o.value), ["", "local", "embed"]);
  await fireEvent.change(embedding, { target: { value: "embed" } });
  await fireEvent.click(screen.getByLabelText("Let Lucy remember things about you"));
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    { patch: { memory: { embedding_profile: "embed", enabled: false } } },
  ]);
});
