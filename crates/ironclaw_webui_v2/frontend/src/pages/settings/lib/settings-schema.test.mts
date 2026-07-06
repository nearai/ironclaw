import assert from "node:assert/strict";
import { test } from "vitest";

import { RESTART_REQUIRED_KEYS } from "./settings-schema.ts";

test("approval settings apply live without a restart banner", () => {
  assert.equal(RESTART_REQUIRED_KEYS.has("agent.auto_approve_tools"), false);
});
