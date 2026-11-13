// Remote access: other machines can approve their own chats only if you say so, and
// encryption is on unless you turn it off.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import RemoteSection from "./RemoteSection.svelte";

afterEach(cleanup);

async function setUp(remote: Record<string, unknown>) {
  const core = new FakeCore();
  core.answers["config.get"] = {
    path: "/c.toml",
    managed: false,
    config: { defaults: { profile: "local" }, profiles: {}, remote },
  };
  core.answers["keys.status"] = [];
  core.answers["persona.get"] = { name: "lucy", text: "", overridden: false, path: null };
  core.answers["config.update"] = { restart_needed: false };
  const store = new SettingsStore(core);
  await store.load();
  render(RemoteSection, { store });
  return core;
}

test("approving from other machines is off and encryption on until you change them", async () => {
  const core = await setUp({ enabled: true, bind: "100.64.0.7:7077", approvals: "local", tls: true });
  const approve = screen.getByLabelText(/approve their own chats/) as HTMLInputElement;
  const encrypt = screen.getByLabelText(/Encrypt with this core/) as HTMLInputElement;
  assert.ok(!approve.checked);
  assert.ok(encrypt.checked);

  await fireEvent.click(approve);
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [{ patch: { remote: { approvals: "remote" } } }]);
});
