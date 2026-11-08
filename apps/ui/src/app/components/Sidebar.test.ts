// The list of chats: pick one, start a new one, rename or delete, see where something's new.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import Sidebar from "./Sidebar.svelte";

afterEach(cleanup);

const sessions = [
  { id: "01AAA", title: "Plans", created_at_ms: 0, updated_at_ms: 2, message_count: 4 },
  { id: "01BBB", title: null, created_at_ms: 0, updated_at_ms: 1, message_count: 2 },
];

function props(overrides: Record<string, unknown> = {}) {
  return {
    sessions,
    selected: "01AAA",
    unread: new Set(["01BBB"]),
    onSelect: () => {},
    onNew: () => {},
    onRename: () => {},
    onDelete: () => {},
    ...overrides,
  };
}

test("chats are listed by title, untitled ones by a stand-in, with the open one marked", () => {
  render(Sidebar, props());

  const items = screen.getAllByRole("listitem");
  assert.equal(items.length, 2);
  assert.ok(items[0]?.textContent?.includes("Plans"));
  assert.ok(items[1]?.textContent?.includes("Untitled chat"));
  assert.equal(items[0]?.getAttribute("aria-current"), "true");
  assert.ok(screen.getByLabelText("new messages"), "the other chat has something new");
});

test("clicking a chat opens it, and New chat starts one", async () => {
  const picked: string[] = [];
  let fresh = 0;
  render(Sidebar, props({ onSelect: (id: string) => picked.push(id), onNew: () => fresh++ }));

  await fireEvent.click(screen.getByRole("button", { name: /^Untitled chat/ }));
  await fireEvent.click(screen.getByRole("button", { name: "New chat" }));

  assert.deepEqual(picked, ["01BBB"]);
  assert.equal(fresh, 1);
});

test("a chat can be renamed in place", async () => {
  const renamed: string[] = [];
  render(Sidebar, props({ onRename: (id: string, title: string) => renamed.push(`${id}=${title}`) }));

  await fireEvent.click(screen.getByRole("button", { name: "Rename Plans" }));
  const box = screen.getByRole("textbox");
  await fireEvent.input(box, { target: { value: "Weekend plans" } });
  await fireEvent.keyDown(box, { key: "Enter" });

  assert.deepEqual(renamed, ["01AAA=Weekend plans"]);
});

test("deleting asks first", async () => {
  const deleted: string[] = [];
  render(Sidebar, props({ onDelete: (id: string) => deleted.push(id) }));

  await fireEvent.click(screen.getByRole("button", { name: "Delete Plans" }));
  assert.deepEqual(deleted, [], "not until you confirm");
  await fireEvent.click(screen.getByRole("button", { name: "Yes, delete" }));

  assert.deepEqual(deleted, ["01AAA"]);
});
