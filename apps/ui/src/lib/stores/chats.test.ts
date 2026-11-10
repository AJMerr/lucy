// The chats behind the main window: the list in the sidebar and the open chat, kept up to date
// from the core's events, including replies in chats you aren't looking at.

import assert from "node:assert/strict";
import { test } from "vitest";

import type { CoreEvent } from "../protocol.ts";
import type { ConnectionState, Transport } from "../rpc/transport.ts";
import { ChatsStore, type ChatsState } from "./chats.ts";

/** A core played by the test: canned answers per method, and events on demand. */
class FakeCore implements Transport {
  state: ConnectionState = "ready";
  calls: { method: string; params: unknown }[] = [];
  answers: Record<string, unknown | ((params: Record<string, unknown>) => unknown)> = {};
  private listeners = new Set<(event: CoreEvent) => void>();

  async call<T>(method: string, params: unknown = {}): Promise<T> {
    this.calls.push({ method, params });
    const answer = this.answers[method];
    if (answer === undefined) {
      throw new Error(`the test didn't expect ${method}`);
    }
    return (typeof answer === "function" ? answer(params as Record<string, unknown>) : answer) as T;
  }
  onEvent(listener: (event: CoreEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  onState(): () => void {
    return () => {};
  }
  close(): void {}
  emit(event: CoreEvent): void {
    for (const listener of this.listeners) {
      listener(event);
    }
  }
  sent(method: string): unknown[] {
    return this.calls.filter((c) => c.method === method).map((c) => c.params);
  }
}

const summary = (id: string, title: string | null, updated: number) => ({
  id,
  title,
  created_at_ms: 0,
  updated_at_ms: updated,
  message_count: 2,
});
const settings = { profile: "local", plain: false, mode: "assist", level: 1 };

function core(): FakeCore {
  const fake = new FakeCore();
  fake.answers["sessions.list"] = [summary("01OLD", "Old chat", 100), summary("01NEW", null, 200)];
  fake.answers["sessions.load"] = {
    summary: summary("01OLD", "Old chat", 100),
    messages: [
      { role: "user", content: "hi", is_error: false, incomplete: false },
      { role: "assistant", content: "Hey.", is_error: false, incomplete: false },
    ],
  };
  fake.answers["session.settings"] = settings;
  return fake;
}

function watch(store: ChatsStore): () => ChatsState {
  let latest: ChatsState | undefined;
  store.subscribe((state) => {
    latest = state;
  });
  return () => {
    assert.ok(latest);
    return latest;
  };
}

test("the sidebar lists chats, most recent first", async () => {
  const store = new ChatsStore(core());
  const state = watch(store);

  await store.load();

  assert.deepEqual(
    state().sessions.map((s) => s.id),
    ["01NEW", "01OLD"],
  );
  assert.equal(state().selected, null);
});

test("opening a chat shows its history and settings", async () => {
  const fake = core();
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();

  await store.open("01OLD");

  assert.equal(state().selected, "01OLD");
  const chat = state().chats["01OLD"];
  assert.ok(chat);
  assert.deepEqual(chat.transcript.entries, [
    { kind: "you", text: "hi" },
    { kind: "lucy", text: "Hey.", streaming: false },
  ]);
  assert.deepEqual(chat.settings, settings);
});

test("a message you send, and the reply, show up in the open chat", async () => {
  const fake = core();
  fake.answers["chat.send"] = { session_id: "01OLD", turn_id: 7 };
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();
  await store.open("01OLD");

  await store.send("what's new?");
  fake.emit({ type: "user_message", session_id: "01OLD", text: "what's new?" });
  fake.emit({ type: "turn_started", session_id: "01OLD", turn_id: 7 });
  fake.emit({ type: "delta", session_id: "01OLD", turn_id: 7, text: "Not much." });

  assert.deepEqual(fake.sent("chat.send"), [{ session_id: "01OLD", text: "what's new?" }]);
  const chat = state().chats["01OLD"];
  assert.deepEqual(chat?.transcript.entries.slice(2), [
    { kind: "you", text: "what's new?" },
    { kind: "lucy", text: "Not much.", streaming: true },
  ]);
  assert.equal(chat?.busy, true);
  fake.emit({ type: "turn_finished", session_id: "01OLD", turn_id: 7, outcome: "complete" });
  assert.equal(state().chats["01OLD"]?.busy, false);
});

test("sending with no chat open starts a new one and opens it", async () => {
  const fake = core();
  fake.answers["chat.send"] = { session_id: "01FRESH", turn_id: 1 };
  fake.answers["sessions.load"] = { summary: summary("01FRESH", null, 300), messages: [] };
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();

  await store.send("hello");

  assert.deepEqual(fake.sent("chat.send"), [{ text: "hello" }]);
  assert.equal(state().selected, "01FRESH");
  assert.ok(state().sessions.some((s) => s.id === "01FRESH"));
});

test("a reply in a chat you aren't looking at still lands there, marked unread", async () => {
  const fake = core();
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();
  await store.open("01OLD");
  fake.answers["sessions.load"] = { summary: summary("01NEW", null, 200), messages: [] };
  await store.open("01NEW");

  fake.emit({ type: "turn_started", session_id: "01OLD", turn_id: 3 });
  fake.emit({ type: "delta", session_id: "01OLD", turn_id: 3, text: "Done in the background." });
  fake.emit({ type: "turn_finished", session_id: "01OLD", turn_id: 3, outcome: "complete" });

  assert.equal(state().selected, "01NEW");
  assert.deepEqual(state().chats["01OLD"]?.transcript.entries.at(-1), {
    kind: "lucy",
    text: "Done in the background.",
    streaming: false,
  });
  assert.equal(state().chats["01OLD"]?.unread, true);
  await store.open("01OLD");
  assert.equal(state().chats["01OLD"]?.unread, false);
});

test("a chat started somewhere else (a terminal, the sprite) appears in the list", async () => {
  const fake = core();
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();
  fake.answers["sessions.list"] = [
    summary("01OLD", "Old chat", 100),
    summary("01NEW", null, 200),
    summary("01TERM", null, 500),
  ];

  fake.emit({ type: "user_message", session_id: "01TERM", text: "from the terminal" });
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(state().sessions[0]?.id, "01TERM");
});

test("answering an approval tells the core and stops the card asking", async () => {
  const fake = core();
  fake.answers["tool.approve"] = null;
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();
  await store.open("01OLD");
  fake.emit({ type: "turn_started", session_id: "01OLD", turn_id: 4 });
  fake.emit({
    type: "tool_approval",
    session_id: "01OLD",
    turn_id: 4,
    call: { id: "c1", name: "run_command", arguments: { command: "ls" } },
    reason: "runs a command",
  });

  await store.approve("c1", "once");

  assert.deepEqual(fake.sent("tool.approve"), [
    { session_id: "01OLD", call_id: "c1", decision: "once" },
  ]);
  const card = state().chats["01OLD"]?.transcript.entries.at(-1);
  assert.equal(card?.kind === "tool" ? card.state : undefined, "running");
});

test("changing the chat's mode, level, model or plain mode goes to the core", async () => {
  const fake = core();
  fake.answers["session.set_mode"] = { ...settings, mode: "agent" };
  fake.answers["session.set_level"] = { ...settings, mode: "agent", level: 2 };
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();
  await store.open("01OLD");

  await store.setMode("agent");
  await store.setLevel(2);

  assert.deepEqual(fake.sent("session.set_mode"), [{ session_id: "01OLD", mode: "agent" }]);
  assert.deepEqual(fake.sent("session.set_level"), [{ session_id: "01OLD", level: 2 }]);
  assert.deepEqual(state().chats["01OLD"]?.settings, { ...settings, mode: "agent", level: 2 });
});

test("renaming and deleting chats", async () => {
  const fake = core();
  fake.answers["sessions.rename"] = null;
  fake.answers["sessions.delete"] = null;
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();
  await store.open("01OLD");

  await store.rename("01NEW", "Plans");
  assert.equal(state().sessions.find((s) => s.id === "01NEW")?.title, "Plans");

  await store.remove("01OLD");
  assert.deepEqual(
    state().sessions.map((s) => s.id),
    ["01NEW"],
  );
  assert.equal(state().selected, null, "the open chat was the one deleted");
  assert.equal(state().chats["01OLD"], undefined);
});

test("a call the core refuses shows as an error in the chat", async () => {
  const fake = core();
  fake.answers["chat.send"] = () => {
    throw new Error("no config at /x; run `lucy config init`");
  };
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();
  await store.open("01OLD");

  await store.send("hi");

  assert.deepEqual(state().chats["01OLD"]?.transcript.entries.at(-1), {
    kind: "lucy",
    text: "",
    streaming: false,
    error: "no config at /x; run `lucy config init`",
  });
});

test("a reply that was already going when you opened the chat appears in full when it ends", async () => {
  const fake = core();
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();
  await store.open("01OLD");

  // Its start happened before the chat was opened, so these can't be placed...
  fake.emit({ type: "delta", session_id: "01OLD", turn_id: 9, text: "…the end." });
  // ...and when it finishes, the saved chat has the whole reply.
  fake.answers["sessions.load"] = {
    summary: summary("01OLD", "Old chat", 100),
    messages: [
      { role: "user", content: "hi", is_error: false, incomplete: false },
      { role: "assistant", content: "Hey.", is_error: false, incomplete: false },
      { role: "user", content: "tell me a story", is_error: false, incomplete: false },
      { role: "assistant", content: "Once upon a time… the end.", is_error: false, incomplete: false },
    ],
  };
  fake.emit({ type: "turn_finished", session_id: "01OLD", turn_id: 9, outcome: "complete" });
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(state().chats["01OLD"]?.transcript.entries.at(-1), {
    kind: "lucy",
    text: "Once upon a time… the end.",
    streaming: false,
  });
});

// --- Pictures ----------------------------------------------------------------------------------

const png = { media_type: "image/png", data: "iVBORw0KGgo=" };

test("pictures go with your message", async () => {
  const fake = core();
  fake.answers["chat.send"] = { session_id: "01OLD", turn_id: 7 };
  const store = new ChatsStore(fake);
  await store.load();
  await store.open("01OLD");

  await store.send("what's this?", [png]);
  await store.send("", [png]);

  assert.deepEqual(fake.sent("chat.send"), [
    { session_id: "01OLD", text: "what's this?", images: [png] },
    { session_id: "01OLD", text: "", images: [png] },
  ]);
});

test("/image makes a picture straight away instead of sending a message", async () => {
  const fake = core();
  fake.answers["images.generate"] = { session_id: "01FRESH", turn_id: 2 };
  fake.answers["sessions.load"] = { summary: summary("01FRESH", null, 300), messages: [] };
  const store = new ChatsStore(fake);
  const state = watch(store);
  await store.load();

  await store.send("/image a lighthouse at dusk");

  assert.deepEqual(fake.sent("images.generate"), [{ prompt: "a lighthouse at dusk" }]);
  assert.deepEqual(fake.sent("chat.send"), []);
  assert.equal(state().selected, "01FRESH");
});

test("a picture is fetched from the core once, as something an <img> can show", async () => {
  const fake = core();
  fake.answers["images.get"] = png;
  const store = new ChatsStore(fake);

  const first = await store.picture("aa11.png");
  const again = await store.picture("aa11.png");

  assert.equal(first, "data:image/png;base64,iVBORw0KGgo=");
  assert.equal(again, first);
  assert.deepEqual(fake.sent("images.get"), [{ id: "aa11.png" }]);
});
