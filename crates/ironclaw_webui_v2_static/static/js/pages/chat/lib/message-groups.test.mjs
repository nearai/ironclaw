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

test("groupMessages: non-auxiliary messages break tool runs", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity" },
    { id: "m", role: "system", content: "paused" },
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

test("groupMessages: final assistant renders after trailing activity", () => {
  const grouped = groupMessages([
    { id: "u", role: "user", content: "connect notion" },
    {
      id: "m",
      role: "assistant",
      content: "I cannot connect Notion.",
      isFinalReply: true,
    },
    { id: "a", role: "tool_activity", toolName: "notion-get-self" },
    { id: "r", role: "thinking", content: "Need to check the integration." },
  ]);

  assert.equal(grouped.length, 4);
  assert.equal(grouped[0].type, "message");
  assert.equal(grouped[0].message.id, "u");
  assert.equal(grouped[1].type, "tool-run");
  assert.deepEqual(grouped[1].tools.map((t) => t.id), ["a"]);
  assert.equal(grouped[2].type, "message");
  assert.equal(grouped[2].message.id, "r");
  assert.equal(grouped[3].type, "message");
  assert.equal(grouped[3].message.id, "m");
});

test("groupMessages: trailing activity stays separate from earlier tool runs", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "m", role: "assistant", content: "Done.", isFinalReply: true },
    { id: "b", role: "tool_activity", toolName: "grep" },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "tool-run");
  assert.deepEqual(grouped[0].tools.map((t) => t.id), ["a"]);
  assert.equal(grouped[1].type, "tool-run");
  assert.deepEqual(grouped[1].tools.map((t) => t.id), ["b"]);
  assert.equal(grouped[2].type, "message");
  assert.equal(grouped[2].message.id, "m");
});

test("groupMessages: middle assistant stays before its following activity", () => {
  const grouped = groupMessages([
    { id: "m1", role: "assistant", content: "I will check." },
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "m2", role: "assistant", content: "Done." },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "message");
  assert.equal(grouped[0].message.id, "m1");
  assert.equal(grouped[1].type, "tool-run");
  assert.equal(grouped[2].type, "message");
  assert.equal(grouped[2].message.id, "m2");
});

test("groupMessages: middle assistant is not hoisted before final reply arrives", () => {
  const grouped = groupMessages([
    { id: "m", role: "assistant", content: "I will check." },
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "b", role: "tool_activity", toolName: "grep" },
  ]);

  assert.equal(grouped.length, 2);
  assert.equal(grouped[0].type, "message");
  assert.equal(grouped[0].message.id, "m");
  assert.equal(grouped[1].type, "tool-run");
  assert.deepEqual(grouped[1].tools.map((t) => t.id), ["a", "b"]);
});

test("groupMessages: assistant is not hoisted when non-auxiliary message trails", () => {
  const grouped = groupMessages([
    { id: "m", role: "assistant", content: "answer", isFinalReply: true },
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "s", role: "system", content: "later note" },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "message");
  assert.equal(grouped[0].message.id, "m");
  assert.equal(grouped[1].type, "tool-run");
  assert.equal(grouped[2].type, "message");
  assert.equal(grouped[2].message.id, "s");
});

test("groupMessages: toolCalls assistant is not the hoist target", () => {
  const grouped = groupMessages([
    { id: "m", role: "assistant", content: "answer", isFinalReply: true },
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "g", role: "assistant", toolCalls: [{ toolName: "read" }] },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "tool-run");
  assert.deepEqual(grouped[0].tools.map((t) => t.id), ["a"]);
  assert.equal(grouped[1].type, "message");
  assert.equal(grouped[1].message.id, "g");
  assert.equal(grouped[2].type, "message");
  assert.equal(grouped[2].message.id, "m");
});

test("groupMessages: no reordering when timeline has no assistant reply", () => {
  const grouped = groupMessages([
    { id: "u", role: "user", content: "run checks" },
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "b", role: "tool_activity", toolName: "grep" },
  ]);

  assert.equal(grouped.length, 2);
  assert.equal(grouped[0].type, "message");
  assert.equal(grouped[0].message.id, "u");
  assert.equal(grouped[1].type, "tool-run");
  assert.deepEqual(grouped[1].tools.map((t) => t.id), ["a", "b"]);
});
