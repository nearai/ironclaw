import assert from "node:assert/strict";
import test from "node:test";

import { summarizeActivity } from "./activity-summary.js";

test("summarizeActivity: nested toolCalls surface failed and running status", () => {
  const summary = summarizeActivity([
    { id: "r", role: "thinking", content: "checking" },
    {
      id: "g",
      role: "assistant",
      toolCalls: [
        { id: "a", toolStatus: "error" },
        { id: "b", toolStatus: "running" },
      ],
    },
  ]);

  assert.equal(summary.hasError, true);
  assert.equal(summary.label, "Activity - 1 reasoning, 2 tools, 1 failed");
});
