// Reminders and scheduled tasks for the Tasks view (`schedules.*`), kept current when one
// fires or a chat schedules a new one.

import type { Transport } from "../rpc/transport.ts";

/** A reminder or task (`lucy_protocol::rpc::ScheduleInfo`). */
export interface ScheduleInfo {
  id: string;
  kind: "reminder" | "task" | string;
  title: string;
  what: string;
  /** In words, e.g. "weekdays at 08:00". */
  when: string;
  when_spec: unknown;
  level: number;
  profile: string;
  paused: boolean;
  next_run_ms: number | null;
  last_run_ms: number | null;
}

export interface TasksState {
  loaded: boolean;
  schedules: ScheduleInfo[];
  /** The core starts at login, so these run even when nothing else started it. */
  alwaysOn: boolean;
  /** Why the last change was refused. */
  error: string | null;
}

type Change = Partial<Pick<ScheduleInfo, "paused" | "what" | "title" | "level" | "profile">>;

export class TasksStore {
  private readonly core: Transport;
  private state: TasksState = { loaded: false, schedules: [], alwaysOn: true, error: null };
  private readonly subscribers = new Set<(state: TasksState) => void>();

  constructor(core: Transport) {
    this.core = core;
    core.onEvent((event) => {
      // A reminder fired or a task ran (next run moved), or a chat scheduled something.
      const scheduled =
        event.type === "tool_finished" && typeof event.output === "string" && event.output.startsWith("Scheduled ");
      if (this.state.loaded && (event.type === "notification" || scheduled)) {
        this.load().catch(() => {});
      }
    });
  }

  subscribe(run: (state: TasksState) => void): () => void {
    this.subscribers.add(run);
    run(this.state);
    return () => this.subscribers.delete(run);
  }

  async load(): Promise<void> {
    const list = await this.core.call<{ schedules: ScheduleInfo[]; always_on: boolean }>(
      "schedules.list",
      {},
    );
    this.update({ loaded: true, schedules: list.schedules, alwaysOn: list.always_on });
  }

  async change(id: string, change: Change): Promise<void> {
    try {
      const changed = await this.core.call<ScheduleInfo>("schedules.update", { id, ...change });
      this.update({
        error: null,
        schedules: this.state.schedules.map((s) => (s.id === id ? changed : s)),
      });
    } catch (err) {
      this.update({ error: err instanceof Error ? err.message : String(err) });
    }
  }

  async remove(id: string): Promise<void> {
    try {
      await this.core.call("schedules.delete", { id });
      this.update({ error: null, schedules: this.state.schedules.filter((s) => s.id !== id) });
    } catch (err) {
      this.update({ error: err instanceof Error ? err.message : String(err) });
    }
  }

  private update(change: Partial<TasksState>): void {
    this.state = { ...this.state, ...change };
    for (const run of this.subscribers) {
      run(this.state);
    }
  }
}
