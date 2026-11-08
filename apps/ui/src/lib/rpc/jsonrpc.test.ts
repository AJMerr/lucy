// JSON-RPC to the core over a socket (a WebSocket on phones and in browsers), driven here by a
// fake socket so the tests see exactly what's sent and decide what comes back.

import assert from "node:assert/strict";
import { test } from "vitest";

import { type Socket, JsonRpcConnection, RpcError } from "./jsonrpc.ts";

/** A socket the test plays the core's side of. */
class FakeSocket implements Socket {
  sent: Record<string, unknown>[] = [];
  private message: (text: string) => void = () => {};
  private closed: () => void = () => {};

  send(text: string): void {
    this.sent.push(JSON.parse(text));
  }
  onMessage(handler: (text: string) => void): void {
    this.message = handler;
  }
  onClose(handler: () => void): void {
    this.closed = handler;
  }
  close(): void {
    this.closed();
  }

  /** The core says something. */
  reply(message: Record<string, unknown>): void {
    this.message(JSON.stringify({ jsonrpc: "2.0", ...message }));
  }
  /** The last request with this method. */
  last(method: string): Record<string, unknown> {
    const found = this.sent.findLast((m) => m.method === method);
    assert.ok(found, `nothing sent with method ${method}`);
    return found;
  }
  /** Answer the handshake the connection starts with. */
  greet(): void {
    this.reply({ id: this.last("hello").id, result: { protocol_version: 5, core_version: "0.4.0", pid: 1 } });
  }
}

/** Let the connection react to what just arrived. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("replies are matched to their calls, whatever order they come back in", async () => {
  const socket = new FakeSocket();
  const rpc = new JsonRpcConnection(socket, "lucy-ui test");
  socket.greet();
  await settle();

  const first = rpc.call("sessions.list", {});
  const second = rpc.call("profiles.list", {});
  await settle();
  socket.reply({ id: socket.last("profiles.list").id, result: ["profiles"] });
  socket.reply({ id: socket.last("sessions.list").id, result: ["sessions"] });

  assert.deepEqual(await first, ["sessions"]);
  assert.deepEqual(await second, ["profiles"]);
});

test("the handshake goes first, and calls made before it wait for it", async () => {
  const socket = new FakeSocket();
  const rpc = new JsonRpcConnection(socket, "lucy-ui test");

  const early = rpc.call("sessions.list", {});
  await settle();
  assert.deepEqual(
    socket.sent.map((m) => m.method),
    ["hello"],
    "nothing else until the core says hello back",
  );
  assert.deepEqual(socket.sent[0]?.params, { protocol_version: 5, client: "lucy-ui test" });

  socket.greet();
  await settle();
  socket.reply({ id: socket.last("sessions.list").id, result: [] });
  assert.deepEqual(await early, []);
});

test("an error from the core rejects the call with its message and code", async () => {
  const socket = new FakeSocket();
  const rpc = new JsonRpcConnection(socket, "test");
  socket.greet();
  await settle();

  const refused = rpc.call("config.update", { patch: {} });
  await settle();
  socket.reply({
    id: socket.last("config.update").id,
    error: { code: -32000, message: "remote clients can't change settings" },
  });

  await assert.rejects(refused, (err: unknown) => {
    assert.ok(err instanceof RpcError);
    assert.equal(err.code, -32000);
    assert.equal(err.message, "remote clients can't change settings");
    return true;
  });
});

test("events from the core reach every listener", async () => {
  const socket = new FakeSocket();
  const rpc = new JsonRpcConnection(socket, "test");
  const heard: unknown[] = [];
  rpc.onEvent((event) => heard.push(event));
  const stop = rpc.onEvent((event) => heard.push(["second", event.type]));

  socket.reply({ method: "event", params: { type: "delta", session_id: "s", turn_id: 1, text: "Hi" } });
  stop();
  socket.reply({ method: "event", params: { type: "turn_finished", session_id: "s", turn_id: 1, outcome: "complete" } });

  assert.deepEqual(heard, [
    { type: "delta", session_id: "s", turn_id: 1, text: "Hi" },
    ["second", "delta"],
    { type: "turn_finished", session_id: "s", turn_id: 1, outcome: "complete" },
  ]);
});

test("losing the connection fails waiting calls, says so, and refuses new ones", async () => {
  const socket = new FakeSocket();
  const rpc = new JsonRpcConnection(socket, "test");
  socket.greet();
  await settle();
  let dropped = 0;
  rpc.onClose(() => dropped++);

  const waiting = rpc.call("chat.send", { text: "hi" });
  await settle();
  socket.close();

  await assert.rejects(waiting, /lost the connection/);
  assert.equal(dropped, 1);
  await assert.rejects(rpc.call("sessions.list"), /lost the connection/);
});

test("a core that refuses the handshake fails every call with its reason", async () => {
  const socket = new FakeSocket();
  const rpc = new JsonRpcConnection(socket, "test");
  socket.reply({
    id: socket.last("hello").id,
    error: { code: -32000, message: "protocol 5 isn't supported (core speaks 4)" },
  });

  await assert.rejects(rpc.call("sessions.list"), /core speaks 4/);
});
