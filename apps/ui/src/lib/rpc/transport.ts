// How the screens reach Lucy's core. They only see `Transport`: calls, events and whether
// the core is reachable. On the desktop that's the Tauri bridge (tauri.ts); on a phone or in
// a browser it's a WebSocket to the core's remote access, kept open by `SocketTransport`.

import type { CoreEvent } from "../protocol.ts";
import { JsonRpcConnection, RpcError, type Socket } from "./jsonrpc.ts";

export type ConnectionState = "connecting" | "ready" | "lost";

export interface Transport {
  readonly state: ConnectionState;
  /** Call a core method; rejects with an `RpcError`. */
  call<T = unknown>(method: string, params?: unknown): Promise<T>;
  /** Every event the core sends (all chats). */
  onEvent(listener: (event: CoreEvent) => void): () => void;
  onState(listener: (state: ConnectionState) => void): () => void;
  close(): void;
}

export interface SocketOptions {
  /** How this front end introduces itself, e.g. "lucy-ui 0.4.0". */
  client: string;
  /** Wait before reconnecting (doubles up to 30 s while the core stays away). */
  retryMs?: number;
}

/** A transport over sockets from `open`, reopened whenever one drops. */
export class SocketTransport implements Transport {
  state: ConnectionState = "connecting";
  private readonly open: () => Promise<Socket>;
  private readonly options: Required<SocketOptions>;
  private connection: Promise<JsonRpcConnection>;
  private readonly events = new Set<(event: CoreEvent) => void>();
  private readonly states = new Set<(state: ConnectionState) => void>();
  private closed = false;
  private retry: number;

  constructor(open: () => Promise<Socket>, options: SocketOptions) {
    this.open = open;
    this.options = { retryMs: 1000, ...options };
    this.retry = this.options.retryMs;
    this.connection = this.connect();
    // Failures surface through calls and state; nobody awaits this one directly.
    this.connection.catch(() => {});
  }

  async call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    const connection = await this.connection;
    return connection.call<T>(method, params);
  }

  onEvent(listener: (event: CoreEvent) => void): () => void {
    this.events.add(listener);
    return () => this.events.delete(listener);
  }

  onState(listener: (state: ConnectionState) => void): () => void {
    this.states.add(listener);
    return () => this.states.delete(listener);
  }

  close(): void {
    this.closed = true;
    this.connection.then((connection) => connection.close()).catch(() => {});
  }

  private async connect(): Promise<JsonRpcConnection> {
    for (;;) {
      try {
        const socket = await this.open();
        const connection = new JsonRpcConnection(socket, this.options.client);
        connection.onEvent((event) => {
          for (const listener of this.events) {
            listener(event);
          }
        });
        await connection.call("subscribe", {});
        connection.onClose(() => this.dropped());
        this.retry = this.options.retryMs;
        this.setState("ready");
        return connection;
      } catch (err) {
        // A refused handshake won't get better by retrying.
        if (err instanceof RpcError && err.code !== 0) {
          this.setState("lost");
          throw err;
        }
        if (this.closed) {
          throw new RpcError(0, "closed");
        }
        await new Promise((resolve) => setTimeout(resolve, this.retry));
        this.retry = Math.min(this.retry * 2 || 1, 30_000);
      }
    }
  }

  private dropped(): void {
    if (this.closed) {
      return;
    }
    this.setState("lost");
    this.connection = this.connect();
    this.connection.catch(() => {});
  }

  private setState(state: ConnectionState): void {
    if (this.state === state) {
      return;
    }
    this.state = state;
    for (const listener of this.states) {
      listener(state);
    }
  }
}

/** Open a WebSocket to the core's remote access (`ws://host:7077/rpc`) with its token. */
export function webSocket(url: string, token: string): () => Promise<Socket> {
  return () =>
    new Promise((resolve, reject) => {
      // Browsers can't set headers on a WebSocket, so the token rides as a subprotocol.
      const ws = new WebSocket(url, ["lucy", `lucy-token.${token}`]);
      ws.onopen = () =>
        resolve({
          send: (text) => ws.send(text),
          onMessage: (handler) => {
            ws.onmessage = (event) => handler(String(event.data));
          },
          onClose: (handler) => {
            ws.onclose = () => handler();
          },
          close: () => ws.close(),
        });
      ws.onerror = () => reject(new RpcError(0, `couldn't reach Lucy's core at ${url}`));
    });
}
