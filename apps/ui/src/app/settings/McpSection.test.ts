// MCP servers: add, change and remove tool servers, and sign in to remote ones.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import McpSection from "./McpSection.svelte";

afterEach(cleanup);

async function setUp(servers: Record<string, unknown> = {}) {
  const core = new FakeCore();
  core.answers["config.get"] = {
    path: "/c.toml",
    managed: false,
    config: { defaults: {}, profiles: {}, ...(Object.keys(servers).length ? { mcp: { servers } } : {}) },
  };
  core.answers["keys.status"] = [];
  core.answers["persona.get"] = { name: "lucy", text: "", overridden: false, path: null };
  core.answers["config.update"] = { restart_needed: false };
  const store = new SettingsStore(core);
  await store.load();
  render(McpSection, { store });
  return core;
}

test("a program server is added with its command, arguments and a key-filled variable", async () => {
  const core = await setUp();

  await fireEvent.input(screen.getByLabelText("New server name"), { target: { value: "github" } });
  await fireEvent.click(screen.getByRole("button", { name: "Add a program" }));
  const server = screen.getByRole("group", { name: "github" });
  await fireEvent.input(within(server).getByLabelText("Command"), { target: { value: "npx" } });
  await fireEvent.input(within(server).getByLabelText("Arguments, one per line"), {
    target: { value: "-y\n@modelcontextprotocol/server-github" },
  });
  await fireEvent.input(within(server).getByLabelText(/Variables from the keyring/), {
    target: { value: "GITHUB_TOKEN=github" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    {
      patch: {
        mcp: {
          servers: {
            github: {
              command: "npx",
              args: ["-y", "@modelcontextprotocol/server-github"],
              env_keys: { GITHUB_TOKEN: "github" },
            },
          },
        },
      },
    },
  ]);
});

test("a server can be turned off or removed", async () => {
  const core = await setUp({
    files: { command: "mcp-files" },
    old: { command: "mcp-old" },
  });

  await fireEvent.click(within(screen.getByRole("group", { name: "files" })).getByLabelText("On"));
  await fireEvent.click(screen.getByRole("button", { name: "Remove old" }));
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  assert.deepEqual(core.sent("config.update"), [
    { patch: { mcp: { servers: { files: { enabled: false }, old: null } } } },
  ]);
});

test("a remote server offers to sign in", async () => {
  const core = await setUp({ linear: { url: "https://mcp.linear.app/mcp" } });
  core.answers["mcp.login"] = null;

  await fireEvent.click(screen.getByRole("button", { name: "Sign in to linear" }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(core.sent("mcp.login"), [{ name: "linear" }]);
  assert.ok(screen.getByText(/Signed in/));
});
