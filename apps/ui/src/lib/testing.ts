// A core played by tests: canned answers per method (or functions of the params, which may
// throw to refuse), every call recorded, and events sent on demand.

import type { CoreEvent } from "./protocol.ts";
import type { ConnectionState, Transport } from "./rpc/transport.ts";

export class FakeCore implements Transport {
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
  /** The params of every call to `method`, in order. */
  sent(method: string): unknown[] {
    return this.calls.filter((c) => c.method === method).map((c) => c.params);
  }
}
