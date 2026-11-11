// What the sprite shows: a pure function from the core's events (for her own chat only; the
// Rust side filters the rest) and your actions to her animation, speech bubble, input box and
// approval request. Timers come in as `tick`s, so this stays testable.

import { describeCall } from "../lib/calls.ts";
import type { Attached } from "../lib/pictures.ts";
import type { CoreEvent, ImageRef } from "../lib/protocol.ts";

export type { CoreEvent };

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
  /** Pictures she made during the reply. */
  pictures?: ImageRef[];
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
  /** Pictures waiting to go with your next message (dropped on her). */
  attached: Attached[];
  /** Reminders and finished tasks that came in while she was replying. */
  waiting: { title: string; body: string }[];
}

export type Input =
  | { type: "event"; event: CoreEvent; now: number }
  | { type: "open_input" }
  | { type: "close_input" }
  /** Pictures dropped on her, to go with your next message. */
  | { type: "attached"; pictures: Attached[] }
  /** You sent a message. */
  | { type: "sent" }
  /** You answered the approval request. */
  | { type: "answered" }
  | { type: "chat_window"; open: boolean }
  | { type: "dismiss" }
  | { type: "tick"; now: number }
  /** A reminder, or a scheduled task finishing. */
  | { type: "notification"; title: string; body: string; now: number }
  /** A word in place of a reply, e.g. after /clear. */
  | { type: "note"; text: string; now: number }
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
    attached: [],
    waiting: [],
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
        attached: [],
        animation: view.turnId === null && view.animation === "listening" ? "idle" : view.animation,
      };
    case "sent":
      return {
        ...view,
        inputOpen: false,
        animation: "thinking",
        bubble: null,
        approval: null,
        attached: [],
      };
    case "attached":
      return {
        ...view,
        inputOpen: true,
        attached: [...view.attached, ...input.pictures],
        animation: view.turnId === null ? "listening" : view.animation,
      };
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
        settings.bubbleTimeoutMs > 0 &&
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
    case "note":
      return { ...view, animation: "idle", bubble: { text: input.text, done: true, doneAt: input.now } };
    case "notification": {
      const note = { title: input.title, body: input.body };
      // Don't cut her off: it goes under her reply once she's done.
      if (view.turnId !== null || view.approval !== null) {
        return { ...view, waiting: [...view.waiting, note] };
      }
      return {
        ...view,
        animation: "happy",
        bubble: { text: notes([note]), done: true, doneAt: input.now },
      };
    }
    case "event": {
      const next = onEvent(view, input.event, input.now);
      const shown = view.chatOpen ? { ...next, bubble: null, approval: null } : next;
      // Notifications that waited for her reply go under it (or alone, if she said nothing).
      if (input.event.type !== "turn_finished" || shown.waiting.length === 0) {
        return shown;
      }
      const reply = shown.bubble?.text ?? "";
      const text = [reply, notes(shown.waiting)].filter((t) => t !== "").join("\n\n");
      return {
        ...shown,
        waiting: [],
        bubble: { ...(shown.bubble ?? { done: true, doneAt: input.now }), text },
      };
    }
  }
}

/** Notifications as bubble text: each title in bold, then its body. */
function notes(waiting: { title: string; body: string }[]): string {
  return waiting.map((n) => `**${n.title}**\n\n${n.body}`).join("\n\n");
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
        bubble: { ...keptPictures(view.bubble), text, done: false },
      };
    }
    case "tool_finished": {
      const made = (Array.isArray(event.images) ? (event.images as ImageRef[]) : []).filter(
        (image) => image.made === true,
      );
      if (made.length === 0) {
        return view;
      }
      const pictures = [...(view.bubble?.pictures ?? []), ...made];
      return { ...view, bubble: { text: view.bubble?.text ?? "", done: false, pictures } };
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
      const kept = keptPictures(view.bubble);
      const bubble: Bubble | null =
        text === "" && !failed && kept.pictures === undefined
          ? null
          : {
              ...kept,
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

/** The pictures already in `bubble`, to carry into its next version. */
function keptPictures(bubble: Bubble | null): { pictures?: ImageRef[] } {
  const pictures = bubble?.pictures ?? [];
  return pictures.length > 0 ? { pictures } : {};
}
