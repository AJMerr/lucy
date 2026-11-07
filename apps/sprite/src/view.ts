// What the sprite shows: a pure function from the core's events (for her own chat only; the
// Rust side filters the rest) and your actions to her animation, speech bubble, input box and
// approval request. Timers come in as `tick`s, so this stays testable.

import { describeCall } from "./calls.ts";

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
  /** The chat window is open: replies and approvals show there instead. */
  chatOpen: boolean;
}

export type Input =
  | { type: "event"; event: CoreEvent; now: number }
  | { type: "open_input" }
  | { type: "close_input" }
  /** You sent a message. */
  | { type: "sent" }
  /** You answered the approval request. */
  | { type: "answered" }
  | { type: "chat_window"; open: boolean }
  | { type: "dismiss" }
  | { type: "tick"; now: number }
  /** Something went wrong outside a reply, e.g. the message couldn't be sent. */
  | { type: "problem"; message: string; now: number };

export interface Settings {
  bubbleTimeoutMs: number;
}

export function initialView(): View {
  return {
    animation: "idle",
    inputOpen: false,
    bubble: null,
    approval: null,
    turnId: null,
    chatOpen: false,
  };
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
    case "chat_window":
      return {
        ...view,
        chatOpen: input.open,
        bubble: input.open ? null : view.bubble,
        approval: input.open ? null : view.approval,
      };
    case "dismiss":
      return view.bubble?.done ? { ...view, bubble: null } : view;
    case "tick": {
      const bubble = view.bubble;
      const fades =
        bubble !== null &&
        bubble.done &&
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
        bubble: { text: "", done: true, error: input.message, doneAt: input.now },
      };
    case "event": {
      const next = onEvent(view, input.event, input.now);
      return view.chatOpen ? { ...next, bubble: null, approval: null } : next;
    }
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
        bubble: { text, done: false },
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
