// Splitting a pasted command line the way a shell would.

import assert from "node:assert/strict";
import { test } from "vitest";

import { splitCommand } from "./shellwords.ts";

test("words, quotes and escapes", () => {
  assert.deepEqual(splitCommand(`npx -y "a b" 'c d' e\\ f`).words, ["npx", "-y", "a b", "c d", "e f"]);
});

test("leading NAME=value words are variables", () => {
  assert.deepEqual(splitCommand("A=1 B='two words' uvx tool --x"), {
    env: { A: "1", B: "two words" },
    words: ["uvx", "tool", "--x"],
  });
});

test("an unclosed quote is an error", () => {
  assert.throws(() => splitCommand(`npx "oops`), /quote/);
});
