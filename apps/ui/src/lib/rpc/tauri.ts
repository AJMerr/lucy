// The desktop transport: calls go to the app's Rust side (bridge.rs), which keeps a connection
// to the core on this machine (started if needed, or the remote one after `lucy remote
// connect`) and passes its events up. No network and no token involved.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { CoreEvent } from "../protocol.ts";
import { RpcError } from "./jsonrpc.ts";
import type { ConnectionState, Transport } from "./transport.ts";

export class TauriTransport implements Transport {
  state: ConnectionState = "connecting";
  private readonly events = new Set<(event: CoreEvent) => void>();
  private readonly states = new Set<(state: ConnectionState) => void>();
  private readonly unlisten: Promise<(() => void)[]>;

  constructor() {
    this.unlisten = Promise.all([
      listen<CoreEvent>("rpc-event", (event) => {
        for (const listener of this.events) {
          listener(event.payload);
        }
      }),
      listen<ConnectionState>("rpc-state", (event) => this.setState(event.payload)),
    ]);
  }

  async call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    await this.unlisten;
    try {
      const result = await invoke<T>("rpc_call", { method, params });
      this.setState("ready");
      return result;
    } catch (err) {
      // The bridge answers with the core's error message, or a lost connection.
      throw new RpcError(-32000, typeof err === "string" ? err : String(err));
    }
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
    this.unlisten.then((fns) => fns.forEach((fn) => fn())).catch(() => {});
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
