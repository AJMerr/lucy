// The mic button: hold it to talk (she listens until you let go), or click it once (she
// listens until you go quiet). Only there when voice is on.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test, vi } from "vitest";

import Composer from "./Composer.svelte";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function setUp(withVoice = true) {
  const calls: string[] = [];
  render(Composer, {
    busy: false,
    onSend: () => {},
    onStop: () => {},
    ...(withVoice
      ? {
          talk: {
            start: (hold: boolean) => calls.push(hold ? "start (hold)" : "start"),
            stop: () => calls.push("stop"),
          },
        }
      : {}),
  });
  return calls;
}

test("holding the mic talks until you let go", async () => {
  vi.useFakeTimers();
  const calls = setUp();
  const mic = screen.getByRole("button", { name: "Talk (hold, or click)" });

  await fireEvent.pointerDown(mic);
  vi.advanceTimersByTime(1500);
  await fireEvent.pointerUp(mic);

  assert.deepEqual(calls, ["start (hold)", "stop"]);
});

test("a quick click listens until you go quiet", async () => {
  vi.useFakeTimers();
  const calls = setUp();
  const mic = screen.getByRole("button", { name: "Talk (hold, or click)" });

  await fireEvent.pointerDown(mic);
  vi.advanceTimersByTime(120);
  await fireEvent.pointerUp(mic);

  assert.deepEqual(calls, ["start (hold)", "start"]);
});

test("without voice there's no mic", () => {
  setUp(false);

  assert.equal(screen.queryByRole("button", { name: "Talk (hold, or click)" }), null);
});
