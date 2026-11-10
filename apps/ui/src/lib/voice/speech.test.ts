// What she says aloud: her reply's prose, without code, tables or link addresses, a sentence at
// a time as it streams in.

import assert from "node:assert/strict";
import { test } from "vitest";

import { SpeechStream, speakable } from "./speech.ts";

test("markup is dropped and links keep their words", () => {
  assert.equal(
    speakable("**Done.** See [the docs](https://example.com) and run `ls`.\n\n- one\n- two"),
    "Done. See the docs and run ls. one. two.",
  );
});

test("headings and numbered lists read as sentences", () => {
  assert.equal(speakable("## Plan\n1. Pack\n2. Go!"), "Plan. Pack. Go!");
});

test("code blocks and tables are left in the chat, said once each", () => {
  const reply =
    "Here's the fix:\n\n```rust\nfn main() {}\n```\n\nAnd the numbers:\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nThat's all.";
  assert.equal(
    speakable(reply),
    "Here's the fix: I've put the code in the chat. And the numbers: There's a table in the chat. That's all.",
  );
});

test("a bare address isn't read out", () => {
  assert.equal(speakable("It's at https://lucy.dev/docs today."), "It's at a link today.");
});

test("snake_case words keep their underscores", () => {
  assert.equal(speakable("Set read_typed on."), "Set read_typed on.");
});

test("sentences come out as soon as they're complete, and the rest at the end", () => {
  const stream = new SpeechStream();

  assert.deepEqual(stream.feed("Hello there"), []);
  assert.deepEqual(stream.feed("Hello there. How are"), ["Hello there."]);
  assert.deepEqual(stream.feed("Hello there. How are you? I'm"), ["How are you?"]);
  assert.deepEqual(stream.finish("Hello there. How are you? I'm fine"), ["I'm fine"]);
});

test("a code block still being written isn't spoken until it's closed", () => {
  const stream = new SpeechStream();

  assert.deepEqual(stream.feed("Try this. ```\nrm -rf"), ["Try this."]);
  assert.deepEqual(stream.feed("Try this. ```\nrm -rf build\n``` Then"), [
    "I've put the code in the chat.",
  ]);
  assert.deepEqual(stream.finish("Try this. ```\nrm -rf build\n``` Then rebuild."), [
    "Then rebuild.",
  ]);
});
