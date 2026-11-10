// The voice pipeline, with a pretend engine: what you say goes to her as a message, her reply
// is spoken a sentence at a time, in order, and talking to her or Stop cuts her off.

import assert from "node:assert/strict";
import { test } from "vitest";

import type { CoreEvent } from "../protocol.ts";
import { type VoiceEngine, VoiceController } from "./controller.ts";

class FakeEngine implements VoiceEngine {
  calls: string[] = [];
  async listen(hold: boolean): Promise<void> {
    this.calls.push(hold ? "listen (hold)" : "listen");
  }
  async stopListening(): Promise<void> {
    this.calls.push("stop listening");
  }
  async say(text: string): Promise<void> {
    this.calls.push(`say: ${text}`);
  }
  async hush(): Promise<void> {
    this.calls.push("hush");
  }
  said(): string[] {
    return this.calls.filter((c) => c.startsWith("say: ")).map((c) => c.slice(5));
  }
}

const session = "01CHAT";
const started = (turn_id: number): CoreEvent => ({ type: "turn_started", session_id: session, turn_id });
const delta = (turn_id: number, text: string): CoreEvent => ({
  type: "delta",
  session_id: session,
  turn_id,
  text,
});
const finished = (turn_id: number, outcome = "complete"): CoreEvent => ({
  type: "turn_finished",
  session_id: session,
  turn_id,
  outcome,
});

function setUp(readTyped = false) {
  const engine = new FakeEngine();
  const sent: string[] = [];
  const voice = new VoiceController(engine, {
    send: async (text) => {
      sent.push(text);
    },
    readTyped: () => readTyped,
  });
  return { engine, sent, voice };
}

test("what you said is sent, and her reply is spoken sentence by sentence", async () => {
  const { engine, sent, voice } = setUp();

  await voice.heard("what's the weather like");
  voice.onEvent(started(1));
  voice.onEvent(delta(1, "Sunny and "));
  voice.onEvent(delta(1, "warm. Take a hat"));
  voice.onEvent(delta(1, "! Back by six"));
  voice.onEvent(finished(1));

  assert.deepEqual(sent, ["what's the weather like"]);
  assert.deepEqual(engine.said(), ["Sunny and warm.", "Take a hat!", "Back by six"]);
});

test("replies to what you typed stay quiet unless you ask for them to be read", async () => {
  const quiet = setUp(false);
  quiet.voice.typed();
  quiet.voice.onEvent(started(1));
  quiet.voice.onEvent(delta(1, "Hi."));
  quiet.voice.onEvent(finished(1));
  assert.deepEqual(quiet.engine.said(), []);

  const aloud = setUp(true);
  aloud.voice.typed();
  aloud.voice.onEvent(started(2));
  aloud.voice.onEvent(delta(2, "Hi."));
  aloud.voice.onEvent(finished(2));
  assert.deepEqual(aloud.engine.said(), ["Hi."]);
});

test("only the reply to your message is spoken, not other chats' or later ones", async () => {
  const { engine, voice } = setUp();

  await voice.heard("hello");
  voice.onEvent(started(1));
  voice.onEvent(delta(1, "Hey."));
  voice.onEvent(finished(1));
  voice.onEvent(started(2));
  voice.onEvent(delta(2, "Something from a terminal."));
  voice.onEvent(finished(2));

  assert.deepEqual(engine.said(), ["Hey."]);
});

test("talking to her cuts her off and starts listening", async () => {
  const { engine, voice } = setUp();
  await voice.heard("tell me a story");
  voice.onEvent(started(1));
  voice.onEvent(delta(1, "Once upon a time. There was"));

  await voice.talk(true);
  voice.onEvent(delta(1, " a netrunner. The end."));

  assert.deepEqual(engine.calls, [
    "say: Once upon a time.",
    "hush",
    "listen (hold)",
  ]);
});

test("Stop hushes her, and a cancelled reply says no more", async () => {
  const { engine, voice } = setUp();
  await voice.heard("go on");
  voice.onEvent(started(1));
  voice.onEvent(delta(1, "One. Two. Thr"));

  voice.onEvent(finished(1, "cancelled"));

  assert.deepEqual(engine.calls, ["say: One.", "say: Two.", "hush"]);
});
