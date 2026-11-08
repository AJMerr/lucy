// Persona: edit your own copy, or go back to the built-in one.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import PersonaSection from "./PersonaSection.svelte";

afterEach(cleanup);

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("editing the persona saves your version, and reset brings back the built-in one", async () => {
  const core = new FakeCore();
  core.answers["config.get"] = { path: "/c.toml", managed: false, config: { defaults: {}, profiles: {} } };
  core.answers["keys.status"] = [];
  core.answers["persona.get"] = { name: "lucy", text: "Built in.", overridden: false, path: "/p/lucy.md" };
  core.answers["persona.set"] = null;
  core.answers["persona.reset"] = null;
  const store = new SettingsStore(core);
  await store.load();
  render(PersonaSection, { store });
  const box = screen.getByLabelText("Persona") as HTMLTextAreaElement;
  assert.equal(box.value, "Built in.");
  assert.ok(screen.getByText(/uses the built-in/));

  await fireEvent.input(box, { target: { value: "Mine." } });
  core.answers["persona.get"] = { name: "lucy", text: "Mine.", overridden: true, path: "/p/lucy.md" };
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await settle();
  assert.deepEqual(core.sent("persona.set"), [{ text: "Mine." }]);
  assert.ok(screen.getByText(/uses your version/));

  core.answers["persona.get"] = { name: "lucy", text: "Built in.", overridden: false, path: "/p/lucy.md" };
  await fireEvent.click(screen.getByRole("button", { name: "Reset to the built-in persona" }));
  await settle();
  assert.equal(core.sent("persona.reset").length, 1);
  assert.equal((screen.getByLabelText("Persona") as HTMLTextAreaElement).value, "Built in.");
});
