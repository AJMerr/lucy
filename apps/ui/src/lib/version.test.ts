// Spotting a core that's older than this app: it keeps running the old version until restarted.

import assert from "node:assert/strict";
import { test } from "vitest";

import { olderCore } from "./version.ts";

test("only an older core is reported", () => {
  assert.equal(olderCore("0.9.0", "0.10.0"), true);
  assert.equal(olderCore("0.10.0", "0.10.0"), false);
  assert.equal(olderCore("0.11.0", "0.10.0"), false);
  assert.equal(olderCore("1.0.0", "0.10.0"), false);
  assert.equal(olderCore("v0.10.0", "0.10.1"), true);
  assert.equal(olderCore("dev", "0.10.0"), false, "a version it can't read isn't reported");
});
