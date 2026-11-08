// Which frame the sprite shows at a given moment: frame timing, loops, one-shot states that
// hand over to another, and switching states.

import { test } from "vitest";
import assert from "node:assert/strict";

import { Animator, type States } from "./animator.ts";

const states: States = {
  idle: { frames: 4, fps: 2, loop: true },
  thinking: { frames: 3, fps: 10, loop: true },
  talking: { frames: 2, fps: 6, loop: true },
  happy: { frames: 2, fps: 4, loop: false, then: "idle" },
  error: { frames: 3, fps: 10, loop: false },
};

test("starts idle on its first frame", () => {
  const animator = new Animator(states, 1000);

  assert.deepEqual(animator.frame(1000), { state: "idle", index: 0 });
});

test("advances at the state's fps and loops", () => {
  const animator = new Animator(states, 0);

  assert.deepEqual(animator.frame(499), { state: "idle", index: 0 });
  assert.deepEqual(animator.frame(500), { state: "idle", index: 1 });
  assert.deepEqual(animator.frame(1600), { state: "idle", index: 3 });
  assert.deepEqual(animator.frame(2000), { state: "idle", index: 0 });
  assert.deepEqual(animator.frame(2750), { state: "idle", index: 1 });
});

test("switching state starts its animation from the first frame", () => {
  const animator = new Animator(states, 0);
  animator.frame(1700);

  animator.play("thinking", 1700);

  assert.deepEqual(animator.frame(1700), { state: "thinking", index: 0 });
  assert.deepEqual(animator.frame(1800), { state: "thinking", index: 1 });
});

test("playing the current state again doesn't restart it", () => {
  const animator = new Animator(states, 0);
  animator.play("talking", 0);

  animator.play("talking", 200);

  assert.deepEqual(animator.frame(200), { state: "talking", index: 1 });
});

test("a one-shot state plays once, then hands over to `then`", () => {
  const animator = new Animator(states, 0);
  animator.play("happy", 0);

  assert.deepEqual(animator.frame(250), { state: "happy", index: 1 });
  assert.deepEqual(animator.frame(499), { state: "happy", index: 1 });
  // happy's two frames take 500 ms; idle starts there.
  assert.deepEqual(animator.frame(500), { state: "idle", index: 0 });
  assert.deepEqual(animator.frame(1000), { state: "idle", index: 1 });
});

test("a one-shot state without `then` holds its last frame", () => {
  const animator = new Animator(states, 0);
  animator.play("error", 0);

  assert.deepEqual(animator.frame(5000), { state: "error", index: 2 });
});

test("states the pack doesn't have show idle", () => {
  const animator = new Animator(states, 0);
  animator.play("listening", 0);

  assert.deepEqual(animator.frame(500), { state: "idle", index: 1 });
});

test("catches up after a long pause without losing its place", () => {
  const animator = new Animator(states, 0);
  animator.play("happy", 0);

  // An hour later (the window was hidden): happy ended long ago, idle kept looping.
  const hour = 60 * 60 * 1000;
  assert.deepEqual(animator.frame(hour + 750), { state: "idle", index: 0 });
});
