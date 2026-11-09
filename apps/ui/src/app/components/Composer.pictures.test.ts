// Pictures in the composer: pasted, dropped or attached, shown until sent, sent with the text.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import type { ImageData } from "../../lib/protocol.ts";
import Composer from "./Composer.svelte";

afterEach(cleanup);

/** "ABC" as a PNG file (the bytes don't matter here). */
const picture = () => new File(["ABC"], "shot.png", { type: "image/png" });
const png: ImageData = { media_type: "image/png", data: "QUJD" };

function setup() {
  const sent: { text: string; images: ImageData[] }[] = [];
  const result = render(Composer, {
    busy: false,
    onSend: (text: string, images: ImageData[]) => sent.push({ text, images }),
    onStop: () => {},
  });
  return { sent, ...result };
}

test("a pasted picture is attached and goes with your message", async () => {
  const { sent } = setup();
  const box = screen.getByRole("textbox");

  await fireEvent.paste(box, { clipboardData: { files: [picture()] } });
  await waitFor(() => screen.getByRole("img", { name: "shot.png" }));
  await fireEvent.input(box, { target: { value: "what's this?" } });
  await fireEvent.keyDown(box, { key: "Enter" });

  assert.deepEqual(sent, [{ text: "what's this?", images: [png] }]);
  assert.equal(screen.queryByRole("img"), null, "cleared once sent");
});

test("a picture can be sent on its own, and attached with the button", async () => {
  const { sent, container } = setup();
  const chooser = container.querySelector("input[type=file]") as HTMLInputElement;

  await fireEvent.change(chooser, { target: { files: [picture()] } });
  await waitFor(() => screen.getByRole("img", { name: "shot.png" }));
  await fireEvent.click(screen.getByRole("button", { name: "Send" }));

  assert.deepEqual(sent, [{ text: "", images: [png] }]);
});

test("a dropped picture is attached, and can be taken off again", async () => {
  const { sent, container } = setup();
  const form = container.querySelector("form") as HTMLFormElement;

  await fireEvent.drop(form, { dataTransfer: { files: [picture()] } });
  await waitFor(() => screen.getByRole("img", { name: "shot.png" }));
  await fireEvent.click(screen.getByRole("button", { name: "Remove shot.png" }));
  await fireEvent.click(screen.getByRole("button", { name: "Send" }));

  assert.equal(screen.queryByRole("img"), null);
  assert.deepEqual(sent, []);
});

test("a file that isn't a picture is turned away, saying why", async () => {
  setup();

  await fireEvent.paste(screen.getByRole("textbox"), {
    clipboardData: { files: [new File(["%PDF"], "doc.pdf", { type: "application/pdf" })] },
  });

  await waitFor(() => screen.getByText(/doc\.pdf isn't a picture/));
  assert.equal(screen.queryByRole("img"), null);
});
