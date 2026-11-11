// The chat window's transcript: a pure function from her saved chat (when the window opens)
// and the core's live events to the list of entries it shows.

import { describeCall } from "./calls.ts";
import type { CoreEvent, ImageRef } from "./protocol.ts";

/** A saved message, as `sessions.load` returns it (`lucy_protocol::rpc::StoredMessage`). */
export interface StoredMessage {
  role: string;
  content: string;
  /** Left out when there are none. */
  tool_calls?: { id: string; name: string; arguments: Record<string, unknown> }[];
  tool_call_id?: string;
  is_error: boolean;
  incomplete: boolean;
  /** Pictures with it; left out when there are none. */
  images?: ImageRef[];
  /** Not a message but a marker: `cleared` (/clear) or `summary` (its text in `content`). */
  marker?: string;
}

export type Entry =
  | { kind: "you"; text: string; /** Pictures you sent with it. */ images?: ImageRef[] }
  | {
      kind: "lucy";
      text: string;
      /** Still arriving. */
      streaming: boolean;
      /** Why the reply failed. */
      error?: string;
      /** A word on how it ended, e.g. "cut off". */
      note?: string;
    }
  | {
      kind: "tool";
      callId: string;
      /** What she wants to do, e.g. "run `git status`". */
      summary: string;
      state: "asking" | "running" | "done" | "failed";
      /** Why it needs your go-ahead (while asking). */
      reason?: string;
      /** What it would change (while asking), if the core sent it. */
      preview?: string;
      /** Pictures it gave back, e.g. one she made. */
      images?: ImageRef[];
    }
  /** Where the chat was cleared, or older messages were summarised (with the summary). */
  | { kind: "marker"; marker: "cleared" | "summary"; text?: string };

export interface Transcript {
  entries: Entry[];
  /** The reply in progress, if any. */
  turnId: number | null;
}

/** The reply in progress when the window opened (kept by `lucy-sprite`; see chat.rs). */
export interface Live {
  turn_id: number;
  /** What she's said so far (since her last tool call). */
  text: string;
  /** The approval request she's waiting on, if any. */
  approval?: CoreEvent;
}

export type TranscriptInput =
  /** Her saved chat, and the reply in progress, when the window opens. */
  | { type: "history"; messages: StoredMessage[]; live: Live | null }
  | { type: "event"; event: CoreEvent }
  /** You (here) answered an approval request. */
  | { type: "answered"; callId: string }
  /** Something went wrong outside a reply, e.g. the message couldn't be sent. */
  | { type: "problem"; message: string };

export function emptyTranscript(): Transcript {
  return { entries: [], turnId: null };
}

export function reduce(transcript: Transcript, input: TranscriptInput): Transcript {
  switch (input.type) {
    case "history": {
      const entries = fromHistory(input.messages);
      const live = input.live;
      if (live === null) {
        return { entries, turnId: null };
      }
      if (live.text !== "") {
        entries.push({ kind: "lucy", text: live.text, streaming: true });
      }
      const resumed = { entries, turnId: live.turn_id };
      return live.approval === undefined ? resumed : onEvent(resumed, live.approval);
    }
    case "event":
      return onEvent(transcript, input.event);
    case "answered":
      return {
        ...transcript,
        entries: transcript.entries.map((entry) =>
          entry.kind === "tool" && entry.callId === input.callId && entry.state === "asking"
            ? { kind: "tool", callId: entry.callId, summary: entry.summary, state: "running" }
            : entry,
        ),
      };
    case "problem":
      return {
        entries: [
          ...transcript.entries,
          { kind: "lucy", text: "", streaming: false, error: input.message },
        ],
        turnId: null,
      };
  }
}

function onEvent(transcript: Transcript, event: CoreEvent): Transcript {
  const entries = [...transcript.entries];
  if (event.type === "user_message") {
    entries.push(you(event.text ?? "", event.images));
    return { ...transcript, entries };
  }
  if (event.type === "turn_started") {
    return { ...transcript, turnId: event.turn_id ?? null };
  }
  // Anything else belongs to a reply; ignore stragglers from an earlier one.
  if (transcript.turnId === null || event.turn_id !== transcript.turnId) {
    return transcript;
  }
  const last = entries.at(-1);
  const streaming = last?.kind === "lucy" && last.streaming ? last : undefined;
  switch (event.type) {
    case "delta":
      if (streaming !== undefined) {
        entries[entries.length - 1] = { ...streaming, text: streaming.text + (event.text ?? "") };
      } else {
        entries.push({ kind: "lucy", text: event.text ?? "", streaming: true });
      }
      break;
    case "tool_approval":
    case "tool_started": {
      const call = event.call;
      if (call === undefined) {
        return transcript;
      }
      if (streaming !== undefined) {
        entries[entries.length - 1] = { ...streaming, streaming: false };
      }
      const asking = event.type === "tool_approval";
      const tool: Entry = {
        kind: "tool",
        callId: call.id,
        summary: describeCall(call.name, call.arguments),
        state: asking ? "asking" : "running",
        ...(asking ? { reason: event.reason ?? "" } : {}),
        ...(asking && event.preview !== undefined ? { preview: event.preview } : {}),
      };
      const at = entries.findIndex((entry) => entry.kind === "tool" && entry.callId === call.id);
      if (at === -1) {
        entries.push(tool);
      } else {
        entries[at] = tool;
      }
      break;
    }
    case "tool_finished": {
      const at = entries.findIndex(
        (entry) => entry.kind === "tool" && entry.callId === event.call_id,
      );
      const tool = entries[at];
      if (tool?.kind === "tool") {
        entries[at] = {
          kind: "tool",
          callId: tool.callId,
          summary: tool.summary,
          state: event.is_error === true ? "failed" : "done",
          ...pictures(event.images),
        };
      }
      break;
    }
    case "turn_finished": {
      const outcome = event.outcome;
      const error = outcome === "failed" ? (event.error ?? "something went wrong") : undefined;
      const note = outcome === "cancelled" ? "stopped" : undefined;
      if (streaming !== undefined) {
        entries[entries.length - 1] = {
          ...streaming,
          streaming: false,
          ...(error === undefined ? {} : { error }),
          ...(note === undefined ? {} : { note }),
        };
      } else if (error !== undefined) {
        entries.push({ kind: "lucy", text: "", streaming: false, error });
      }
      return { entries, turnId: null };
    }
    default:
      return transcript;
  }
  return { ...transcript, entries };
}

function fromHistory(messages: StoredMessage[]): Entry[] {
  const entries: Entry[] = [];
  for (const message of messages) {
    if (message.marker === "cleared") {
      entries.push({ kind: "marker", marker: "cleared" });
      continue;
    }
    if (message.marker === "summary") {
      entries.push({ kind: "marker", marker: "summary", text: message.content });
      continue;
    }
    switch (message.role) {
      case "user":
        entries.push(you(message.content, message.images));
        break;
      case "assistant":
        if (message.content !== "") {
          entries.push({
            kind: "lucy",
            text: message.content,
            streaming: false,
            ...(message.incomplete ? { note: "cut off" } : {}),
          });
        }
        for (const call of message.tool_calls ?? []) {
          entries.push({
            kind: "tool",
            callId: call.id,
            summary: describeCall(call.name, call.arguments),
            state: "done",
          });
        }
        break;
      case "tool": {
        const tool = entries.find(
          (entry) => entry.kind === "tool" && entry.callId === message.tool_call_id,
        );
        if (tool?.kind === "tool" && message.is_error) {
          tool.state = "failed";
        }
        if (tool?.kind === "tool" && (message.images?.length ?? 0) > 0) {
          tool.images = message.images;
        }
        break;
      }
    }
  }
  return entries;
}

function you(text: string, images: unknown): Entry {
  return { kind: "you", text, ...pictures(images) };
}

/** `images` as an entry's pictures, left out when there are none. */
function pictures(images: unknown): { images?: ImageRef[] } {
  return Array.isArray(images) && images.length > 0 ? { images: images as ImageRef[] } : {};
}
