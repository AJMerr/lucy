// JSON-RPC 2.0 to Lucy's core over an open socket: one message per frame, `hello` first,
// replies matched to calls by id, and `event` notifications handed to listeners. The
// WebSocket transport (phones, browsers) uses it; the desktop app's Rust side does the same
// job for its own socket.

import { type CoreEvent, PROTOCOL_VERSION } from "../protocol.ts";

/** What the connection needs from a socket that's already open. */
export interface Socket {
  send(text: string): void;
  onMessage(handler: (text: string) => void): void;
  onClose(handler: () => void): void;
  close(): void;
}

/** An error the core answered with (or a lost connection, code 0). */
export class RpcError extends Error {
  readonly code: number;
  constructor(code: number, message: string) {
    super(message);
    this.name = "RpcError";
    this.code = code;
  }
}

interface Pending {
  resolve: (value: unknown) => void;
  reject: (error: RpcError) => void;
}

export class JsonRpcConnection {
  private readonly socket: Socket;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly listeners = new Set<(event: CoreEvent) => void>();
  private readonly closeListeners = new Set<() => void>();
  private closed = false;
  /** Settles once the core has accepted `hello`; every other call waits for it. */
  readonly ready: Promise<unknown>;

  constructor(socket: Socket, clientName: string) {
    this.socket = socket;
    socket.onMessage((text) => this.receive(text));
    socket.onClose(() => this.lost());
    this.ready = this.send("hello", { protocol_version: PROTOCOL_VERSION, client: clientName });
    // A refused hello rejects every call; don't also report it as unhandled.
    this.ready.catch(() => {});
  }

  /** Call `method` on the core, once the handshake is done. */
  async call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    await this.ready;
    return (await this.send(method, params)) as T;
  }

  /** Hear every event the core sends (after a `subscribe` call). */
  onEvent(listener: (event: CoreEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Hear when the connection drops. */
  onClose(listener: () => void): () => void {
    this.closeListeners.add(listener);
    return () => this.closeListeners.delete(listener);
  }

  close(): void {
    this.socket.close();
  }

  private send(method: string, params: unknown): Promise<unknown> {
    if (this.closed) {
      return Promise.reject(new RpcError(0, "lost the connection to Lucy's core"));
    }
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    });
  }

  private receive(text: string): void {
    let message: {
      id?: number;
      method?: string;
      params?: unknown;
      result?: unknown;
      error?: { code: number; message: string };
    };
    try {
      message = JSON.parse(text);
    } catch {
      return;
    }
    if (message.id === undefined || message.id === null) {
      if (message.method === "event") {
        for (const listener of this.listeners) {
          listener(message.params as CoreEvent);
        }
      }
      return;
    }
    const pending = this.pending.get(message.id);
    if (pending === undefined) {
      return;
    }
    this.pending.delete(message.id);
    if (message.error) {
      pending.reject(new RpcError(message.error.code, message.error.message));
    } else {
      pending.resolve(message.result ?? null);
    }
  }

  private lost(): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    for (const { reject } of this.pending.values()) {
      reject(new RpcError(0, "lost the connection to Lucy's core"));
    }
    this.pending.clear();
    for (const listener of this.closeListeners) {
      listener();
    }
  }
}
