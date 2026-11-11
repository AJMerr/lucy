// The Tasks list: reminders and scheduled tasks she's set up, with when they run next; pause,
// resume, change or delete each.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { TasksStore } from "../../lib/stores/tasks.ts";
import { FakeCore } from "../../lib/testing.ts";
import Tasks from "./Tasks.svelte";

afterEach(cleanup);

const summary = {
  id: "01AAA",
  kind: "task",
  title: "Morning summary",
  what: "Summarise my email and calendar",
  when: "weekdays at 08:00",
  when_spec: { weekdays: "08:00" },
  level: 1,
  profile: "local",
  paused: false,
  next_run_ms: new Date(2026, 10, 11, 8, 0).getTime(),
  last_run_ms: null,
};
const reminder = {
  ...summary,
  id: "01BBB",
  kind: "reminder",
  title: "Call Mom",
  what: "Call Mom",
  when: "once on Tue 10 Nov 2026 at 17:00",
  when_spec: { at: "2026-11-10T17:00" },
};

async function setUp(alwaysOn = false) {
  const core = new FakeCore();
  let schedules = [summary, reminder];
  core.answers["schedules.list"] = () => ({ schedules, always_on: alwaysOn });
  core.answers["schedules.update"] = (params: Record<string, unknown>) => {
    schedules = schedules.map((s) => (s.id === params.id ? { ...s, ...params } : s));
    return schedules.find((s) => s.id === params.id);
  };
  core.answers["schedules.delete"] = (params: Record<string, unknown>) => {
    schedules = schedules.filter((s) => s.id !== params.id);
    return null;
  };
  const store = new TasksStore(core);
  await store.load();
  render(Tasks, { store, profiles: ["local", "claude"], onClose: () => {} });
  return core;
}

test("each one shows what it is, when it runs and its level", async () => {
  await setUp();

  const task = screen.getByRole("group", { name: "Morning summary" });
  assert.ok(within(task).getByText("Summarise my email and calendar"));
  assert.ok(within(task).getByText(/weekdays at 08:00/));
  assert.equal((within(task).getByLabelText("Level") as HTMLSelectElement).value, "1");
  const remind = screen.getByRole("group", { name: "Call Mom" });
  assert.equal(within(remind).queryByLabelText("Level"), null, "reminders have no level");
});

test("it says when the core won't run them unless it's running", async () => {
  await setUp(false);
  assert.ok(screen.getByText(/doesn't start at login/));
  cleanup();
  await setUp(true);
  assert.equal(screen.queryByText(/doesn't start at login/), null);
});

test("pausing and resuming", async () => {
  const core = await setUp();
  const task = screen.getByRole("group", { name: "Morning summary" });

  await fireEvent.click(within(task).getByRole("button", { name: "Pause" }));
  await waitFor(() => within(task).getByRole("button", { name: "Resume" }));
  await fireEvent.click(within(task).getByRole("button", { name: "Resume" }));

  await waitFor(() =>
    assert.deepEqual(core.sent("schedules.update"), [
      { id: "01AAA", paused: true },
      { id: "01AAA", paused: false },
    ]),
  );
});

test("a task's level and model can be changed", async () => {
  const core = await setUp();
  const task = screen.getByRole("group", { name: "Morning summary" });

  await fireEvent.change(within(task).getByLabelText("Level"), { target: { value: "2" } });
  await fireEvent.change(within(task).getByLabelText("Model"), { target: { value: "claude" } });

  await waitFor(() =>
    assert.deepEqual(core.sent("schedules.update"), [
      { id: "01AAA", level: 2 },
      { id: "01AAA", profile: "claude" },
    ]),
  );
});

test("what it does can be reworded", async () => {
  const core = await setUp();
  const task = screen.getByRole("group", { name: "Morning summary" });

  await fireEvent.click(within(task).getByRole("button", { name: "Edit" }));
  await fireEvent.input(within(task).getByRole("textbox", { name: "What" }), {
    target: { value: "Summarise my email" },
  });
  await fireEvent.click(within(task).getByRole("button", { name: "Save" }));

  await waitFor(() =>
    assert.deepEqual(core.sent("schedules.update"), [{ id: "01AAA", what: "Summarise my email" }]),
  );
});

test("deleting asks first", async () => {
  const core = await setUp();
  const remind = screen.getByRole("group", { name: "Call Mom" });

  await fireEvent.click(within(remind).getByRole("button", { name: "Delete" }));
  assert.deepEqual(core.sent("schedules.delete"), []);
  await fireEvent.click(within(remind).getByRole("button", { name: "Yes, delete" }));

  await waitFor(() => assert.deepEqual(core.sent("schedules.delete"), [{ id: "01BBB" }]));
  await waitFor(() => assert.equal(screen.queryByRole("group", { name: "Call Mom" }), null));
});

test("with nothing scheduled, it says how to make one", async () => {
  const core = new FakeCore();
  core.answers["schedules.list"] = { schedules: [], always_on: true };
  const store = new TasksStore(core);
  await store.load();
  render(Tasks, { store, profiles: [], onClose: () => {} });

  assert.ok(screen.getByText(/remind me at 5/));
});
