// What the sprite shows: a pure function from the core's events (for her own chat only; the
// Rust side filters the rest) and your actions to her animation, speech bubble, input box and
// approval request. Timers come in as `tick`s, so this stays testable.

/** A core event, as the protocol serializes it (`lucy_protocol::rpc::Event`). */
export interface CoreEvent {
  type: string;
  turn_id?: number;
  text?: string;
  call?: { id: string; name: string; arguments: Record<string, unknown> };
  reason?: string;
  preview?: string;
  outcome?: string;
  error?: string;
  [key: string]: unknown;
}

export type Animation = "idle" | "listening" | "thinking" | "talking" | "happy" | "error";

export interface Bubble {
  /** Her reply so far. */
  text: string;
  /** The reply has ended (it may then fade). */
  done: boolean;
  /** Why the reply failed, shown under the text. */
  error?: string;
  /** When it ended, for fading. */
  doneAt?: number;
  /** Showing all of it; stays until dismissed. */
  expanded: boolean;
}

export interface Approval {
  callId: string;
  /** What she wants to do, e.g. "run `git status`". */
  summary: string;
  /** Why it needs your go-ahead. */
  reason: string;
  /** What it would change (a diff), if the core sent one. */
  preview?: string;
}

export interface View {
  animation: Animation;
  inputOpen: boolean;
  bubble: Bubble | null;
  approval: Approval | null;
  /** The reply in progress, if any. */
  turnId: number | null;
}

export type Input =
  | { type: "event"; event: CoreEvent; now: number }
  | { type: "open_input" }
  | { type: "close_input" }
  /** You sent a message. */
  | { type: "sent" }
  /** You answered the approval request. */
  | { type: "answered" }
  | { type: "expand" }
  | { type: "dismiss" }
  | { type: "tick"; now: number }
  /** Something went wrong outside a reply, e.g. the message couldn't be sent. */
  | { type: "problem"; message: string; now: number };

export interface Settings {
  bubbleTimeoutMs: number;
}

export function initialView(): View {
  return { animation: "idle", inputOpen: false, bubble: null, approval: null, turnId: null };
}

export function reduce(view: View, input: Input, settings: Settings): View {
  switch (input.type) {
    case "open_input":
      return {
        ...view,
        inputOpen: true,
        animation: view.turnId === null ? "listening" : view.animation,
      };
    case "close_input":
      return {
        ...view,
        inputOpen: false,
        animation: view.turnId === null && view.animation === "listening" ? "idle" : view.animation,
      };
    case "sent":
      return { ...view, inputOpen: false, animation: "thinking", bubble: null, approval: null };
    case "answered":
      return { ...view, approval: null, animation: "thinking" };
    case "expand":
      return view.bubble === null ? view : { ...view, bubble: { ...view.bubble, expanded: true } };
    case "dismiss":
      return view.bubble?.done ? { ...view, bubble: null } : view;
    case "tick": {
      const bubble = view.bubble;
      const fades =
        bubble !== null &&
        bubble.done &&
        !bubble.expanded &&
        view.approval === null &&
        bubble.doneAt !== undefined &&
        input.now >= bubble.doneAt + settings.bubbleTimeoutMs;
      return fades ? { ...view, bubble: null } : view;
    }
    case "problem":
      return {
        ...view,
        turnId: null,
        approval: null,
        animation: "error",
        bubble: { text: "", done: true, error: input.message, doneAt: input.now, expanded: false },
      };
    case "event":
      return onEvent(view, input.event, input.now);
  }
}

function onEvent(view: View, event: CoreEvent, now: number): View {
  if (event.type === "turn_started") {
    return {
      ...view,
      turnId: event.turn_id ?? null,
      animation: "thinking",
      bubble: null,
      approval: null,
    };
  }
  // Anything else belongs to a reply; ignore stragglers from an earlier one.
  if (event.turn_id !== view.turnId || view.turnId === null) {
    return view;
  }
  switch (event.type) {
    case "delta": {
      const text = (view.bubble?.text ?? "") + (event.text ?? "");
      return {
        ...view,
        animation: "talking",
        bubble: { text, done: false, expanded: view.bubble?.expanded ?? false },
      };
    }
    case "tool_approval": {
      const call = event.call;
      if (call === undefined) {
        return view;
      }
      return {
        ...view,
        animation: "listening",
        approval: {
          callId: call.id,
          summary: describeCall(call.name, call.arguments),
          reason: event.reason ?? "",
          ...(event.preview === undefined ? {} : { preview: event.preview }),
        },
      };
    }
    case "tool_started":
      return { ...view, animation: "thinking", approval: null };
    case "turn_finished": {
      const outcome = event.outcome;
      const text = view.bubble?.text ?? "";
      const failed = outcome === "failed";
      const bubble: Bubble | null =
        text === "" && !failed
          ? null
          : {
              text,
              done: true,
              doneAt: now,
              expanded: view.bubble?.expanded ?? false,
              ...(failed ? { error: event.error ?? "something went wrong" } : {}),
            };
      const animation: Animation =
        outcome === "complete" || outcome === "capped"
          ? "happy"
          : failed
            ? "error"
            : "idle";
      return { ...view, turnId: null, approval: null, animation, bubble };
    }
    default:
      return view;
  }
}

/** A tool call in a few words, e.g. "run `git status`" or "edit notes.md". */
function describeCall(name: string, args: Record<string, unknown>): string {
  const arg = (key: string) => (typeof args[key] === "string" ? (args[key] as string) : "");
  switch (name) {
    case "run_command":
      return `run \`${arg("command")}\``;
    case "write_file":
      return `write ${arg("path")}`;
    case "edit_file":
      return `edit ${arg("path")}`;
    case "read_file":
      return `read ${arg("path")}`;
    case "list_dir":
      return `look in ${arg("path") || "."}`;
    case "search_files":
      return `search for "${arg("pattern")}"`;
    case "fetch_url":
      return `open ${arg("url")}`;
    default:
      return `use ${name}`;
  }
}
