// One entry in a chat: your message, her reply (Markdown), or a tool she ran or wants to run.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import Entry from "./Entry.svelte";

afterEach(cleanup);

test("her reply renders Markdown, and anything that looks like HTML stays text", () => {
  const { container } = render(Entry, {
    entry: {
      kind: "lucy",
      text: "**Done.** Run `ls`.\n\n<img src=x onerror=alert(1)>",
      streaming: false,
    },
    onDecide: () => {},
  });

  assert.equal(container.querySelector("strong")?.textContent, "Done.");
  assert.equal(container.querySelector("code")?.textContent, "ls");
  assert.equal(container.querySelector("img"), null);
  assert.ok(container.textContent?.includes("<img src=x onerror=alert(1)>"));
});

test("your message is plain text", () => {
  render(Entry, { entry: { kind: "you", text: "**not bold**" }, onDecide: () => {} });

  assert.ok(screen.getByText("**not bold**"));
});

test("a failed reply says why", () => {
  render(Entry, {
    entry: { kind: "lucy", text: "", streaming: false, error: "the model is down" },
    onDecide: () => {},
  });

  assert.ok(screen.getByText("the model is down"));
});

test("an approval request offers Allow, Always and Deny, and says which you picked", async () => {
  const picked: string[] = [];
  render(Entry, {
    entry: {
      kind: "tool",
      callId: "c1",
      summary: "run `rm -rf build`",
      state: "asking",
      reason: "runs a command",
    },
    onDecide: (callId: string, decision: string) => picked.push(`${callId}:${decision}`),
  });

  assert.ok(screen.getByText("rm -rf build"));
  assert.ok(screen.getByText("runs a command"));
  await fireEvent.click(screen.getByRole("button", { name: "Always" }));
  await fireEvent.click(screen.getByRole("button", { name: "Deny" }));

  assert.deepEqual(picked, ["c1:always", "c1:deny"]);
});

test("a change she wants to make can be shown before you decide", async () => {
  const { container } = render(Entry, {
    entry: {
      kind: "tool",
      callId: "c2",
      summary: "edit notes.md",
      state: "asking",
      reason: "changes a file",
      preview: "- old\n+ new",
    },
    onDecide: () => {},
  });

  assert.equal(container.querySelector("pre"), null);
  await fireEvent.click(screen.getByRole("button", { name: "Show changes" }));
  assert.equal(container.querySelector("pre")?.textContent, "- old\n+ new");
});

test("a finished tool call is one quiet line, marked if it failed", () => {
  render(Entry, {
    entry: { kind: "tool", callId: "c3", summary: "read notes.md", state: "failed" },
    onDecide: () => {},
  });

  assert.ok(screen.getByText(/failed/));
  assert.equal(screen.queryByRole("button", { name: "Allow" }), null);
});
