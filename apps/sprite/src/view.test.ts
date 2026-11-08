// What the sprite shows, given what the core says and what you do: her animation, the speech
// bubble, the input box and approval requests.

import { test } from "node:test";
import assert from "node:assert/strict";

import { type CoreEvent, type Input, type View, initialView, reduce } from "./view.ts";

const settings = { bubbleTimeoutMs: 12_000 };
const session = "01SPRITE";

function run(inputs: Input[], from: View = initialView()): View {
  return inputs.reduce((view, input) => reduce(view, input, settings), from);
}

function event(event: CoreEvent, now = 0): Input {
  return { type: "event", event, now };
}

const started = (turn_id: number) =>
  event({ type: "turn_started", session_id: session, turn_id, profile: "p", model: "m" });
const delta = (turn_id: number, text: string) =>
  event({ type: "delta", session_id: session, turn_id, text });
const finished = (turn_id: number, outcome: string, now = 0, error?: string) =>
  event(
    {
      type: "turn_finished",
      session_id: session,
      turn_id,
      outcome,
      ...(error === undefined ? {} : { error }),
    },
    now,
  );
const approval = (turn_id: number, name: string, args: Record<string, unknown>) =>
  event({
    type: "tool_approval",
    session_id: session,
    turn_id,
    call: { id: `call-${name}`, name, arguments: args },
    reason: "runs a command",
  });

test("she starts idle, with nothing to say", () => {
  const view = initialView();

  assert.equal(view.animation, "idle");
  assert.equal(view.bubble, null);
  assert.equal(view.inputOpen, false);
});

test("she listens while you type, and goes back to idle if you don't send", () => {
  const typing = run([{ type: "open_input" }]);
  assert.equal(typing.animation, "listening");
  assert.equal(typing.inputOpen, true);

  const closed = run([{ type: "close_input" }], typing);
  assert.equal(closed.animation, "idle");
  assert.equal(closed.inputOpen, false);
});

test("sending closes the box, clears her last reply and makes her think", () => {
  const replied = run([started(1), delta(1, "Old news."), finished(1, "complete")]);

  const view = run([{ type: "open_input" }, { type: "sent" }], replied);

  assert.equal(view.animation, "thinking");
  assert.equal(view.inputOpen, false);
  assert.equal(view.bubble, null);
});

test("her reply streams into the bubble while she talks", () => {
  const view = run([{ type: "sent" }, started(1), delta(1, "Hello, "), delta(1, "choom.")]);

  assert.equal(view.animation, "talking");
  assert.equal(view.bubble?.text, "Hello, choom.");
  assert.equal(view.bubble?.done, false);
});

test("a finished reply makes her happy, and the bubble fades after the timeout", () => {
  const done = run([started(1), delta(1, "Done."), finished(1, "complete", 1_000)]);
  assert.equal(done.animation, "happy");
  assert.equal(done.bubble?.done, true);

  const still = run([{ type: "tick", now: 12_999 }], done);
  assert.equal(still.bubble?.text, "Done.");

  const faded = run([{ type: "tick", now: 13_000 }], done);
  assert.equal(faded.bubble, null);
});

test("with fading off, her reply stays up until you send the next message", () => {
  const stays = { bubbleTimeoutMs: 0 };
  const play = (inputs: Input[], from: View) =>
    inputs.reduce((view, input) => reduce(view, input, stays), from);
  const done = play([started(1), delta(1, "Read me."), finished(1, "complete", 1_000)], initialView());

  const hourLater = play([{ type: "tick", now: 3_600_000 }], done);
  assert.equal(hourLater.bubble?.text, "Read me.");

  const next = play([{ type: "open_input" }, { type: "sent" }], hourLater);
  assert.equal(next.bubble, null);
});

test("a finished bubble goes when you dismiss it", () => {
  const done = run([started(1), delta(1, "Long story."), finished(1, "complete", 0)]);

  const dismissed = run([{ type: "dismiss" }], done);
  assert.equal(dismissed.bubble, null);
});

test("while the chat window is open her replies go there, not in a bubble, but she still acts", () => {
  const talking = run([started(1), delta(1, "Long story.")]);

  const windowed = run([{ type: "chat_window", open: true }], talking);
  assert.equal(windowed.bubble, null, "the window has it now");

  const later = run(
    [delta(1, " The end."), approval(1, "run_command", { command: "ls" })],
    windowed,
  );
  assert.equal(later.bubble, null);
  assert.equal(later.approval, null, "the window asks instead");
  assert.equal(later.animation, "listening");

  const closed = run([{ type: "chat_window", open: false }, started(2), delta(2, "Hi.")], later);
  assert.equal(closed.bubble?.text, "Hi.");
});

test("a tool that needs your go-ahead shows what she wants to do, and she waits", () => {
  const view = run([started(1), approval(1, "run_command", { command: "git status" })]);

  assert.equal(view.animation, "listening");
  assert.deepEqual(view.approval, {
    callId: "call-run_command",
    summary: "run `git status`",
    reason: "runs a command",
  });

  const answered = run([{ type: "answered" }], view);
  assert.equal(answered.approval, null);
  assert.equal(answered.animation, "thinking");
});

test("approvals describe file changes by path", () => {
  const view = run([started(1), approval(1, "edit_file", { path: "notes.md" })]);

  assert.equal(view.approval?.summary, "edit notes.md");
});

test("using a tool after talking makes her think again", () => {
  const view = run([
    started(1),
    delta(1, "Let me look."),
    event({
      type: "tool_started",
      session_id: session,
      turn_id: 1,
      call: { id: "c1", name: "read_file", arguments: { path: "a.txt" } },
    }),
  ]);

  assert.equal(view.animation, "thinking");
  assert.equal(view.bubble?.text, "Let me look.");
});

test("a failed reply shows the error and her error face", () => {
  const view = run([started(1), finished(1, "failed", 0, "model unreachable")]);

  assert.equal(view.animation, "error");
  assert.equal(view.bubble?.error, "model unreachable");
  assert.equal(view.bubble?.done, true);
});

test("a cancelled reply just goes quiet", () => {
  const view = run([started(1), delta(1, "Once upon"), finished(1, "cancelled")]);

  assert.equal(view.animation, "idle");
  assert.equal(view.bubble?.text, "Once upon");
});

test("leftovers from an earlier reply don't leak into the new one", () => {
  const view = run([started(1), delta(1, "first"), started(2), delta(1, " stale"), delta(2, "second")]);

  assert.equal(view.bubble?.text, "second");
});

test("a message that couldn't be sent shows why", () => {
  const view = run([{ type: "sent" }, { type: "problem", message: "the core isn't running", now: 0 }]);

  assert.equal(view.animation, "error");
  assert.equal(view.bubble?.error, "the core isn't running");
});
