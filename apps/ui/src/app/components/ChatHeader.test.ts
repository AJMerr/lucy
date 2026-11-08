// The open chat's own settings, like `/model`, `/mode`, `/level` and `/plain` in the terminal,
// and `/undo`.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import type { SessionSettings } from "../../lib/stores/chats.ts";
import ChatHeader from "./ChatHeader.svelte";

const assist: SessionSettings = { profile: "local", plain: false, mode: "assist", level: 1 };
const agent: SessionSettings = { ...assist, mode: "agent" };

afterEach(cleanup);

const profiles = [
  { name: "local", provider: "openai_compat", model: "qwen3:14b", is_default: true },
  { name: "claude", provider: "anthropic", model: "claude-opus-5-5", is_default: false },
];

function props(overrides: Record<string, unknown> = {}) {
  return {
    title: "Plans",
    settings: assist,
    profiles,
    busy: false,
    onProfile: () => {},
    onMode: () => {},
    onLevel: () => {},
    onPlain: () => {},
    onUndo: () => {},
    ...overrides,
  };
}

test("it shows the chat's model and mode, and switching them asks the core", async () => {
  const changes: string[] = [];
  render(
    ChatHeader,
    props({
      onProfile: (name: string) => changes.push(`profile=${name}`),
      onMode: (mode: string) => changes.push(`mode=${mode}`),
    }),
  );
  const model = screen.getByLabelText("Model") as HTMLSelectElement;
  assert.equal(model.value, "local");

  await fireEvent.change(model, { target: { value: "claude" } });
  await fireEvent.click(screen.getByRole("button", { name: "Agent" }));

  assert.deepEqual(changes, ["profile=claude", "mode=agent"]);
});

test("the agent level only shows in agent mode", async () => {
  const levels: number[] = [];
  const { rerender } = render(ChatHeader, props({ onLevel: (level: number) => levels.push(level) }));
  assert.equal(screen.queryByLabelText("Level"), null);

  await rerender(
    props({
      settings: agent,
      onLevel: (level: number) => levels.push(level),
    }),
  );
  await fireEvent.change(screen.getByLabelText("Level"), { target: { value: "2" } });

  assert.deepEqual(levels, [2]);
});

test("plain mode and undo", async () => {
  const done: string[] = [];
  render(
    ChatHeader,
    props({
      onPlain: (plain: boolean) => done.push(`plain=${plain}`),
      onUndo: () => done.push("undo"),
    }),
  );

  await fireEvent.click(screen.getByRole("checkbox", { name: "Plain" }));
  await fireEvent.click(screen.getByRole("button", { name: "Undo changes" }));

  assert.deepEqual(done, ["plain=true", "undo"]);
});

test("nothing can be switched while she's replying", () => {
  render(ChatHeader, props({ busy: true }));

  assert.ok((screen.getByLabelText("Model") as HTMLSelectElement).disabled);
  assert.ok((screen.getByRole("button", { name: "Undo changes" }) as HTMLButtonElement).disabled);
});
