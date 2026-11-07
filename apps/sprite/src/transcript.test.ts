// The chat window: her whole chat, from what the core has saved plus what happens live.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  type Entry,
  type StoredMessage,
  type Transcript,
  type TranscriptInput,
  emptyTranscript,
  reduce,
} from "./transcript.ts";
import type { CoreEvent } from "./view.ts";

const session = "01SPRITE";

function run(inputs: TranscriptInput[], from: Transcript = emptyTranscript()): Transcript {
  return inputs.reduce(reduce, from);
}

function event(event: CoreEvent): TranscriptInput {
  return { type: "event", event };
}

const said = (text: string) => event({ type: "user_message", session_id: session, text });
const started = (turn_id: number) =>
  event({ type: "turn_started", session_id: session, turn_id, profile: "p", model: "m" });
const delta = (turn_id: number, text: string) =>
  event({ type: "delta", session_id: session, turn_id, text });
const finished = (turn_id: number, outcome: string, error?: string) =>
  event({
    type: "turn_finished",
    session_id: session,
    turn_id,
    outcome,
    ...(error === undefined ? {} : { error }),
  });
const call = (id: string) => ({ id, name: "run_command", arguments: { command: "ls" } });

function message(role: string, content: string, extra: Partial<StoredMessage> = {}): StoredMessage {
  return { role, content, tool_calls: [], is_error: false, incomplete: false, ...extra };
}

test("the window opens on her chat so far: you, her, and what she ran", () => {
  const transcript = run([
    {
      type: "history",
      messages: [
        message("system", "You are Lucy."),
        message("user", "what's here?"),
        message("assistant", "Let me look.", { tool_calls: [call("c1")] }),
        message("tool", "a.txt\nb.txt", { tool_call_id: "c1" }),
        message("assistant", "Two files."),
        message("user", "and now?"),
        message("assistant", "Half a tho", { incomplete: true }),
      ],
      live: null,
    },
  ]);

  const expected: Entry[] = [
    { kind: "you", text: "what's here?" },
    { kind: "lucy", text: "Let me look.", streaming: false },
    { kind: "tool", callId: "c1", summary: "run `ls`", state: "done" },
    { kind: "lucy", text: "Two files.", streaming: false },
    { kind: "you", text: "and now?" },
    { kind: "lucy", text: "Half a tho", streaming: false, note: "cut off" },
  ];
  assert.deepEqual(transcript.entries, expected);
  assert.equal(transcript.turnId, null);
});

test("what you send and her reply appear as they happen", () => {
  const streaming = run([said("hi"), started(1), delta(1, "Hey, "), delta(1, "choom.")]);
  assert.deepEqual(streaming.entries, [
    { kind: "you", text: "hi" },
    { kind: "lucy", text: "Hey, choom.", streaming: true },
  ]);
  assert.equal(streaming.turnId, 1);

  const done = run([finished(1, "complete")], streaming);
  assert.deepEqual(done.entries.at(-1), { kind: "lucy", text: "Hey, choom.", streaming: false });
  assert.equal(done.turnId, null);
});

test("what she says after running something comes after it", () => {
  const transcript = run([
    said("look"),
    started(1),
    delta(1, "Checking."),
    event({ type: "tool_started", session_id: session, turn_id: 1, call: call("c1") }),
    event({
      type: "tool_finished",
      session_id: session,
      turn_id: 1,
      call_id: "c1",
      output: "boom",
      is_error: true,
    }),
    delta(1, "That failed."),
    finished(1, "complete"),
  ]);

  assert.deepEqual(transcript.entries, [
    { kind: "you", text: "look" },
    { kind: "lucy", text: "Checking.", streaming: false },
    { kind: "tool", callId: "c1", summary: "run `ls`", state: "failed" },
    { kind: "lucy", text: "That failed.", streaming: false },
  ]);
});

test("she asks before running something, and the question goes once you answer", () => {
  const asking = run([
    said("delete it"),
    started(1),
    event({
      type: "tool_approval",
      session_id: session,
      turn_id: 1,
      call: call("c1"),
      reason: "runs a command",
      preview: "rm a.txt",
    }),
  ]);
  assert.deepEqual(asking.entries.at(-1), {
    kind: "tool",
    callId: "c1",
    summary: "run `ls`",
    state: "asking",
    reason: "runs a command",
    preview: "rm a.txt",
  });

  // Answered here or anywhere else (a terminal on the same chat): the buttons go.
  const answered = run([{ type: "answered", callId: "c1" }], asking);
  assert.deepEqual(answered.entries.at(-1), {
    kind: "tool",
    callId: "c1",
    summary: "run `ls`",
    state: "running",
  });
});

test("a failed reply says why, and a stopped one says so", () => {
  const failed = run([said("hi"), started(1), finished(1, "failed", "the model is down")]);
  assert.deepEqual(failed.entries.at(-1), {
    kind: "lucy",
    text: "",
    streaming: false,
    error: "the model is down",
  });

  const stopped = run([started(2), delta(2, "Well"), finished(2, "cancelled")], failed);
  assert.deepEqual(stopped.entries.at(-1), {
    kind: "lucy",
    text: "Well",
    streaming: false,
    note: "stopped",
  });
});

test("opened in the middle of a reply, the window carries on from what she's said so far", () => {
  const transcript = run([
    {
      type: "history",
      messages: [message("user", "tell me a story")],
      live: { turn_id: 4, text: "Once upon " },
    },
    delta(4, "a time"),
  ]);

  assert.deepEqual(transcript.entries, [
    { kind: "you", text: "tell me a story" },
    { kind: "lucy", text: "Once upon a time", streaming: true },
  ]);
  assert.equal(transcript.turnId, 4);
});

test("a message that couldn't be sent shows as an error", () => {
  const transcript = run([{ type: "problem", message: "lost the connection to Lucy's core" }]);

  assert.deepEqual(transcript.entries, [
    { kind: "lucy", text: "", streaming: false, error: "lost the connection to Lucy's core" },
  ]);
  assert.equal(transcript.turnId, null);
});
