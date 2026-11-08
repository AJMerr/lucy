// Where you type: Enter sends, Shift+Enter starts a new line, Stop cuts her reply short.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import Composer from "./Composer.svelte";

afterEach(cleanup);

test("Enter sends what you typed, trimmed, and clears the box", async () => {
  const sent: string[] = [];
  render(Composer, { busy: false, onSend: (text: string) => sent.push(text), onStop: () => {} });
  const box = screen.getByRole("textbox") as HTMLTextAreaElement;

  await fireEvent.input(box, { target: { value: "  what's up?  " } });
  await fireEvent.keyDown(box, { key: "Enter" });

  assert.deepEqual(sent, ["what's up?"]);
  assert.equal(box.value, "");
});

test("Shift+Enter doesn't send, and an empty box never does", async () => {
  const sent: string[] = [];
  render(Composer, { busy: false, onSend: (text: string) => sent.push(text), onStop: () => {} });
  const box = screen.getByRole("textbox");

  await fireEvent.keyDown(box, { key: "Enter" });
  await fireEvent.input(box, { target: { value: "line one" } });
  await fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
  await fireEvent.click(screen.getByRole("button", { name: "Send" }));

  assert.deepEqual(sent, ["line one"]);
});

test("while she's replying, Stop is there and stops her", async () => {
  let stopped = 0;
  const { rerender } = render(Composer, {
    busy: false,
    onSend: () => {},
    onStop: () => stopped++,
  });
  assert.equal(screen.queryByRole("button", { name: "Stop" }), null);

  await rerender({ busy: true, onSend: () => {}, onStop: () => stopped++ });
  await fireEvent.click(screen.getByRole("button", { name: "Stop" }));

  assert.equal(stopped, 1);
});
