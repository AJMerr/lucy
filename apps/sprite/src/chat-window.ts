// The chat window: her whole chat, for replies too long for a bubble. What to show comes from
// `transcript.ts`; this file turns it into DOM and sends what you type.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { renderBubble } from "./bubble.ts";
import type { CoreEvent } from "./view.ts";
import {
  type Entry,
  type Live,
  type StoredMessage,
  type Transcript,
  type TranscriptInput,
  emptyTranscript,
  reduce,
} from "./transcript.ts";

function element<T extends HTMLElement>(selector: string): T {
  const found = document.querySelector<T>(selector);
  if (found === null) {
    throw new Error(`missing ${selector}`);
  }
  return found;
}

const log = element<HTMLElement>("#log");
const empty = element<HTMLDivElement>("#empty");
const ask = element<HTMLFormElement>("#ask");
const input = element<HTMLTextAreaElement>("#ask textarea");
const stop = element<HTMLButtonElement>("#stop");

let transcript: Transcript = emptyTranscript();
/** Approval requests whose changes you've opened. */
const previews = new Set<string>();

function dispatch(change: TranscriptInput): void {
  transcript = reduce(transcript, change);
  render();
}

// --- Drawing ----------------------------------------------------------------------------------

/** What each entry was drawn from, so only changed entries are drawn again. */
let drawn: { key: string; node: HTMLElement }[] = [];
const thinking = document.createElement("div");
thinking.id = "thinking";
thinking.textContent = "…";

function render(): void {
  const stuck = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
  const entries = transcript.entries;
  empty.hidden = entries.length > 0 || transcript.turnId !== null;
  drawn = entries.map((entry, i) => {
    const key = JSON.stringify(entry) + (previews.has(callId(entry)) ? "+" : "");
    const old = drawn[i];
    if (old?.key === key) {
      return old;
    }
    const node = draw(entry);
    if (old === undefined) {
      log.insertBefore(node, thinking.parentNode === log ? thinking : null);
    } else {
      old.node.replaceWith(node);
    }
    return { key, node };
  });
  for (const extra of [...log.querySelectorAll(":scope > .entry")].slice(entries.length)) {
    extra.remove();
  }
  const last = entries.at(-1);
  const waiting =
    transcript.turnId !== null &&
    !(last?.kind === "lucy" && last.streaming) &&
    !(last?.kind === "tool" && last.state === "asking");
  if (waiting) {
    log.append(thinking);
  } else {
    thinking.remove();
  }
  stop.hidden = transcript.turnId === null;
  if (stuck) {
    log.scrollTop = log.scrollHeight;
  }
}

function callId(entry: Entry): string {
  return entry.kind === "tool" ? entry.callId : "";
}

function draw(entry: Entry): HTMLElement {
  const node = document.createElement("div");
  node.className = `entry ${entry.kind}`;
  switch (entry.kind) {
    case "you":
      node.textContent = entry.text;
      break;
    case "lucy": {
      if (entry.text !== "") {
        const shown = renderBubble(entry.text, { maxLines: 0, charsPerLine: 80, expanded: true });
        node.innerHTML = shown.html;
      }
      if (entry.error !== undefined) {
        node.append(line("error", entry.error));
      }
      if (entry.note !== undefined) {
        node.append(line("note", `(${entry.note})`));
      }
      break;
    }
    case "tool": {
      node.classList.add(entry.state);
      // What she runs comes from the model: text nodes only. Backticked parts show as code.
      const summary = document.createElement("div");
      summary.append(entry.state === "asking" ? "Okay to " : "▸ ");
      entry.summary.split("`").forEach((part, i) => {
        if (i % 2 === 0) {
          summary.append(part);
        } else {
          const code = document.createElement("code");
          code.textContent = part;
          summary.append(code);
        }
      });
      if (entry.state === "asking") {
        summary.append("?");
        node.append(summary, line("reason", entry.reason ?? ""));
        if (entry.preview !== undefined && previews.has(entry.callId)) {
          const pre = document.createElement("pre");
          pre.textContent = entry.preview;
          node.append(pre);
        }
        node.append(approvalButtons(entry.callId, entry.preview !== undefined));
      } else {
        const state = document.createElement("span");
        state.className = "state";
        state.textContent = { running: " · running", done: "", failed: " · failed" }[
          entry.state
        ];
        summary.append(state);
        node.append(summary);
      }
      break;
    }
  }
  return node;
}

function line(className: string, text: string): HTMLDivElement {
  const div = document.createElement("div");
  div.className = className;
  div.textContent = text;
  return div;
}

function approvalButtons(id: string, hasPreview: boolean): HTMLDivElement {
  const buttons = document.createElement("div");
  buttons.className = "buttons";
  for (const [decision, label] of [
    ["once", "Allow"],
    ["always", "Always"],
    ["deny", "Deny"],
  ] as const) {
    const button = document.createElement("button");
    button.textContent = label;
    if (decision === "once") {
      button.className = "primary";
    }
    button.addEventListener("click", () => {
      dispatch({ type: "answered", callId: id });
      invoke("approve", { callId: id, decision }).catch((err: unknown) =>
        dispatch({ type: "problem", message: String(err) }),
      );
    });
    buttons.append(button);
  }
  if (hasPreview) {
    const show = document.createElement("button");
    show.className = "link";
    show.textContent = previews.has(id) ? "hide changes" : "show changes";
    show.addEventListener("click", () => {
      if (!previews.delete(id)) {
        previews.add(id);
      }
      render();
    });
    buttons.append(show);
  }
  return buttons;
}

// --- You --------------------------------------------------------------------------------------

function send(): void {
  const message = input.value.trim();
  if (message === "") {
    return;
  }
  input.value = "";
  fit();
  invoke("send", { text: message }).catch((err: unknown) =>
    dispatch({ type: "problem", message: String(err) }),
  );
}

/** Grow the box with what you type, up to its max height. */
function fit(): void {
  input.style.height = "auto";
  input.style.height = `${input.scrollHeight + 2}px`;
}

ask.addEventListener("submit", (event) => {
  event.preventDefault();
  send();
});
input.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
    event.preventDefault();
    send();
  }
});
input.addEventListener("input", fit);
stop.addEventListener("click", () => {
  invoke("cancel").catch((err: unknown) => console.error("lucy-sprite:", err));
});

// --- The core ---------------------------------------------------------------------------------

async function start(): Promise<void> {
  // Events that arrive while her chat loads are already in what loads.
  let loaded = false;
  await listen<CoreEvent>("core-event", (event) => {
    if (loaded) {
      dispatch({ type: "event", event: event.payload });
    }
  });
  await listen("core-disconnected", () => {
    if (transcript.turnId !== null) {
      dispatch({ type: "problem", message: "lost the connection to Lucy's core" });
    }
  });
  try {
    const history = await invoke<{ messages: StoredMessage[]; live: Live | null }>("history");
    dispatch({ type: "history", messages: history.messages, live: history.live });
  } catch (err) {
    dispatch({ type: "problem", message: String(err) });
  }
  loaded = true;
  log.scrollTop = log.scrollHeight;
  input.focus();
}

start().catch((err: unknown) => console.error("lucy-sprite:", err));
