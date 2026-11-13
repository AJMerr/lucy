// The socket transport the screens use on phones and in browsers: it keeps a connection to
// the core, subscribes to events, and reconnects when the connection drops.

import assert from "node:assert/strict";
import { test } from "vitest";

import type { Socket } from "./jsonrpc.ts";
import { type ConnectionState, SocketTransport } from "./transport.ts";

class FakeSocket implements Socket {
  sent: Record<string, unknown>[] = [];
  private message: (text: string) => void = () => {};
  private closed: () => void = () => {};
  send(text: string): void {
    const parsed = JSON.parse(text);
    this.sent.push(parsed);
    // Play a cooperative core: say hello back and accept subscribe.
    if (parsed.method === "hello" || parsed.method === "subscribe") {
      queueMicrotask(() =>
        this.reply({ id: parsed.id, result: parsed.method === "hello" ? { protocol_version: 6 } : null }),
      );
    }
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
  reply(message: Record<string, unknown>): void {
    this.message(JSON.stringify({ jsonrpc: "2.0", ...message }));
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("it connects, subscribes, and picks up again after the connection drops", async () => {
  const sockets: FakeSocket[] = [];
  const states: ConnectionState[] = [];
  const transport = new SocketTransport(
    async () => {
      const socket = new FakeSocket();
      sockets.push(socket);
      return socket;
    },
    { client: "test", retryMs: 0 },
  );
  transport.onState((state) => states.push(state));
  const heard: string[] = [];
  transport.onEvent((event) => heard.push(`${event.type}:${event.text ?? ""}`));
  await settle();
  await settle();

  assert.deepEqual(
    sockets[0]?.sent.map((m) => m.method),
    ["hello", "subscribe"],
  );
  sockets[0]?.reply({ method: "event", params: { type: "delta", text: "one" } });

  sockets[0]?.close();
  await settle();
  await settle();
  await settle();

  assert.equal(sockets.length, 2, "a new connection after the drop");
  assert.deepEqual(
    sockets[1]?.sent.map((m) => m.method),
    ["hello", "subscribe"],
  );
  sockets[1]?.reply({ method: "event", params: { type: "delta", text: "two" } });
  assert.deepEqual(heard, ["delta:one", "delta:two"]);
  assert.deepEqual(states, ["ready", "lost", "ready"]);
  assert.equal(transport.state, "ready");
  transport.close();
});

test("calls go over the current connection", async () => {
  const socket = new FakeSocket();
  const transport = new SocketTransport(async () => socket, { client: "test", retryMs: 0 });

  const listed = transport.call("sessions.list", {});
  await settle();
  await settle();
  const request = socket.sent.find((m) => m.method === "sessions.list");
  assert.ok(request, "the call was sent once connected");
  socket.reply({ id: request.id, result: [{ id: "01A" }] });

  assert.deepEqual(await listed, [{ id: "01A" }]);
  transport.close();
});
