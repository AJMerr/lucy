// The sprite's page: draws her from the pack, shows her replies and approval requests in a
// bubble, and takes your messages in an input box. What to show comes from `view.ts`; this
// file turns that into DOM, animation and window layout.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { Animator, type States } from "./animator.ts";
import { renderBubble } from "../lib/bubble.ts";
import { type CoreEvent, type Input, type View, initialView, reduce } from "./view.ts";

/** The pack as `lucy-sprite` hands it over (see src-tauri/src/pack.rs). */
interface PackView {
  name: string;
  width: number;
  height: number;
  scale: number;
  anchor: string;
  bubble_timeout_secs: number;
  states: Record<string, { frames: string[]; fps: number; loop: boolean; then?: string }>;
}

/** How much text fits in the bubble before "more…". */
const BUBBLE_LINES = 8;
const CHARS_PER_LINE = 36;

function element<T extends HTMLElement>(selector: string): T {
  const found = document.querySelector<T>(selector);
  if (found === null) {
    throw new Error(`missing ${selector}`);
  }
  return found;
}

/**
 * Make WebKit repaint the whole page, not just what changed. Without the GPU renderer (which
 * crashes on some Wayland setups, NVIDIA's included) WebKitGTK repaints only the changed area
 * into alternating buffers, so everything else (the bubble, the input box) is missing from
 * every other frame. A change to the page background too small to see damages it all.
 */
let repaintFlip = false;
let flippedThisFrame = false;
function repaintAll(): void {
  // Once per frame: a second flip in the same frame would undo the first.
  if (flippedThisFrame) {
    return;
  }
  flippedThisFrame = true;
  requestAnimationFrame(() => {
    flippedThisFrame = false;
  });
  repaintFlip = !repaintFlip;
  document.documentElement.style.backgroundColor = repaintFlip
    ? "rgba(0, 0, 0, 0.004)"
    : "rgba(0, 0, 0, 0)";
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("a frame wouldn't load"));
    image.src = src;
  });
}

async function start(): Promise<void> {
  const pack = await invoke<PackView>("pack");
  const frames: Record<string, HTMLImageElement[]> = {};
  const timing: States = {};
  for (const [name, state] of Object.entries(pack.states)) {
    frames[name] = await Promise.all(state.frames.map(loadImage));
    timing[name] = {
      frames: state.frames.length,
      fps: state.fps,
      loop: state.loop,
      ...(state.then === undefined ? {} : { then: state.then }),
    };
  }

  // --- Her figure ---------------------------------------------------------------------------
  const canvas = element<HTMLCanvasElement>("#sprite");
  const width = Math.round(pack.width * pack.scale);
  const height = Math.round(pack.height * pack.scale);
  const ratio = window.devicePixelRatio || 1;
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  canvas.width = Math.round(width * ratio);
  canvas.height = Math.round(height * ratio);
  const context = canvas.getContext("2d");
  if (context === null) {
    throw new Error("no 2D canvas");
  }
  // Pixel art stays crisp when scaled up.
  context.imageSmoothingEnabled = pack.scale * ratio < 1;
  const animator = new Animator(timing, performance.now());
  let drawn = "";
  const draw = (now: number) => {
    const shown = animator.frame(now);
    const key = `${shown.state}:${shown.index}`;
    // Only when the frame changes: every draw repaints the whole page (see repaintAll).
    if (key !== drawn) {
      drawn = key;
      const image = frames[shown.state]?.[shown.index];
      context.clearRect(0, 0, canvas.width, canvas.height);
      if (image !== undefined) {
        context.drawImage(image, 0, 0, canvas.width, canvas.height);
      }
      repaintAll();
    }
    requestAnimationFrame(draw);
  };
  requestAnimationFrame(draw);

  // --- What she shows -----------------------------------------------------------------------
  const settings = { bubbleTimeoutMs: pack.bubble_timeout_secs * 1000 };
  let view: View = initialView();
  let animation = view.animation;
  const dispatch = (input: Input) => {
    view = reduce(view, input, settings);
    render();
  };

  const bubble = element<HTMLDivElement>("#bubble");
  const text = element<HTMLDivElement>("#text");
  const error = element<HTMLDivElement>("#error");
  const more = element<HTMLButtonElement>("#more");
  const close = element<HTMLButtonElement>("#close");
  const approval = element<HTMLDivElement>("#approval");
  const summary = element<HTMLDivElement>("#approval .summary");
  const reason = element<HTMLDivElement>("#approval .reason");
  const preview = element<HTMLPreElement>("#approval pre");
  const show = element<HTMLButtonElement>("#show");
  const ask = element<HTMLFormElement>("#ask");
  const input = element<HTMLInputElement>("#ask input");

  function render(): void {
    if (view.animation !== animation) {
      animation = view.animation;
      animator.play(animation, performance.now());
    }
    const reply = view.bubble;
    bubble.hidden = reply === null && view.approval === null;
    if (reply !== null) {
      const shown = renderBubble(reply.text, {
        maxLines: BUBBLE_LINES,
        charsPerLine: CHARS_PER_LINE,
        expanded: false,
      });
      text.innerHTML = shown.html;
      text.hidden = reply.text === "";
      more.hidden = !shown.more;
      error.hidden = reply.error === undefined;
      error.textContent = reply.error ?? "";
      close.hidden = !reply.done;
    } else {
      text.hidden = true;
      more.hidden = true;
      error.hidden = true;
      close.hidden = true;
    }
    approval.hidden = view.approval === null;
    if (view.approval !== null) {
      // What she wants to run comes from the model: text nodes only, never HTML. Backticked
      // parts (the command) show as code.
      summary.replaceChildren(
        ...`Okay to ${view.approval.summary}?`.split("`").map((part, i) => {
          if (i % 2 === 0) {
            return document.createTextNode(part);
          }
          const code = document.createElement("code");
          code.textContent = part;
          return code;
        }),
      );
      reason.textContent = view.approval.reason;
      show.hidden = view.approval.preview === undefined;
      preview.textContent = view.approval.preview ?? "";
    } else {
      preview.hidden = true;
    }
    const opening = ask.hidden && view.inputOpen;
    ask.hidden = !view.inputOpen;
    if (opening) {
      input.value = "";
      input.focus();
    }
    layout();
    repaintAll();
  }

  // --- Window layout ------------------------------------------------------------------------
  const stage = element<HTMLDivElement>("#stage");
  let lastLayout = "";
  function layout(): void {
    const box = stage.getBoundingClientRect();
    const windowWidth = Math.ceil(Math.max(box.width, width));
    const windowHeight = Math.ceil(Math.max(box.height, height));
    // Everything hangs from the bottom-right corner, so the parts that take clicks are given
    // by their distance from it, which holds at any window size.
    const regions = [bubble, ask, canvas]
      .filter((part) => !part.hidden)
      .map((part) => {
        const rect = part.getBoundingClientRect();
        return {
          right: window.innerWidth - rect.right,
          bottom: window.innerHeight - rect.bottom,
          width: rect.width,
          height: rect.height,
        };
      });
    const args = { width: windowWidth, height: windowHeight, regions, typing: view.inputOpen };
    const key = JSON.stringify(args);
    if (key !== lastLayout) {
      lastLayout = key;
      invoke("set_layout", args).catch((err: unknown) => console.error("lucy-sprite:", err));
    }
  }

  // --- You ----------------------------------------------------------------------------------
  canvas.addEventListener("click", () =>
    dispatch({ type: view.inputOpen ? "close_input" : "open_input" }),
  );
  ask.addEventListener("submit", (event) => {
    event.preventDefault();
    const message = input.value.trim();
    if (message === "") {
      return;
    }
    dispatch({ type: "sent" });
    invoke("send", { text: message }).catch((err: unknown) =>
      dispatch({ type: "problem", message: String(err), now: Date.now() }),
    );
  });
  // Typing changes only the input box; repaint the rest with it (see repaintAll). keydown
  // comes in the same task as the text change, so both land in one frame.
  input.addEventListener("keydown", repaintAll);
  input.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      dispatch({ type: "close_input" });
    }
  });
  // The whole reply (and the rest of her chat) is in the chat window.
  more.addEventListener("click", () => {
    invoke("open_chat").catch((err: unknown) => console.error("lucy-sprite:", err));
  });
  close.addEventListener("click", () => dispatch({ type: "dismiss" }));
  show.addEventListener("click", () => {
    preview.hidden = !preview.hidden;
    layout();
  });
  for (const button of approval.querySelectorAll<HTMLButtonElement>("[data-decision]")) {
    button.addEventListener("click", () => {
      const pending = view.approval;
      if (pending === null) {
        return;
      }
      dispatch({ type: "answered" });
      invoke("approve", { callId: pending.callId, decision: button.dataset.decision }).catch(
        (err: unknown) => dispatch({ type: "problem", message: String(err), now: Date.now() }),
      );
    });
  }

  // --- The core and the CLI -----------------------------------------------------------------
  await listen<CoreEvent>("core-event", (event) =>
    dispatch({ type: "event", event: event.payload, now: Date.now() }),
  );
  await listen("core-disconnected", () => {
    if (view.turnId !== null) {
      dispatch({ type: "problem", message: "lost the connection to Lucy's core", now: Date.now() });
    }
  });
  await listen<boolean>("chat-window", (event) =>
    dispatch({ type: "chat_window", open: event.payload }),
  );
  await listen<string>("sprite-command", (event) => {
    if (event.payload === "ask") {
      dispatch({ type: view.inputOpen ? "close_input" : "open_input" });
    }
  });
  setInterval(() => dispatch({ type: "tick", now: Date.now() }), 500);
  render();
}

start().catch((err: unknown) => console.error("lucy-sprite:", err));
