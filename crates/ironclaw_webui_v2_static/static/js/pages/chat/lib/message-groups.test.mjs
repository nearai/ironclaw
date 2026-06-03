import assert from "node:assert/strict";
import test from "node:test";

import { groupMessages } from "./message-groups.js";

test("groupMessages: consecutive tool_activity messages collapse into one run", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "b", role: "tool_activity", toolName: "grep" },
  ]);

  assert.equal(grouped.length, 1);
  assert.equal(grouped[0].type, "tool-run");
  assert.deepEqual(grouped[0].tools.map((t) => t.id), ["a", "b"]);
});

test("groupMessages: non-tool messages break tool runs", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity" },
    { id: "m", role: "assistant", content: "done" },
    { id: "b", role: "tool_activity" },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "tool-run");
  assert.equal(grouped[1].type, "message");
  assert.equal(grouped[2].type, "tool-run");
});

test("groupMessages: toolCalls-bearing messages are not grouped and break runs", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity" },
    { id: "g", role: "assistant", toolCalls: [{ toolName: "read" }] },
    { id: "b", role: "tool_activity" },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "tool-run");
  assert.equal(grouped[1].type, "message");
  assert.equal(grouped[2].type, "tool-run");
});
