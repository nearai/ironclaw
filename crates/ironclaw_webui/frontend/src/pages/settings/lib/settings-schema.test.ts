import assert from "node:assert/strict";
import { test } from "vitest";

import { RESTART_REQUIRED_KEYS } from "./settings-schema";

test("approval settings apply live without a restart banner", () => {
  assert.equal(RESTART_REQUIRED_KEYS.has("agent.auto_approve_tools"), false);
});

test("unsupported inference settings do not trigger a restart banner", () => {
  for (const key of [
    "embeddings.enabled",
    "embeddings.provider",
    "embeddings.model",
    "llm.temperature",
  ]) {
    assert.equal(RESTART_REQUIRED_KEYS.has(key), false, `${key} should not require restart`);
  }
});
