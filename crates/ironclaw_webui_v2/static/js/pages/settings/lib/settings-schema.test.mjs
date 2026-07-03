import assert from "node:assert/strict";
import test from "node:test";

import { RESTART_REQUIRED_KEYS } from "./settings-schema.js";

test("approval settings apply live without a restart banner", () => {
  assert.equal(RESTART_REQUIRED_KEYS.has("agent.auto_approve_tools"), false);
});
