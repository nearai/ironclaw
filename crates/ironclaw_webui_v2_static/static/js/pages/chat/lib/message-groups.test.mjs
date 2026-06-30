import assert from "node:assert/strict";
import test from "node:test";

import { groupMessages } from "./message-groups.js";

test("groupMessages: consecutive tool_activity messages collapse into one run", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "b", role: "tool_activity", toolName: "grep" },
  ]);

  assert.equal(grouped.length, 1);
  assert.equal(grouped[0].type, "activity-run");
  assert.deepEqual(grouped[0].activity.map((item) => item.id), ["a", "b"]);
});

test("groupMessages: tool activities sort by stable activity order", () => {
  const grouped = groupMessages([
    {
      id: "tool-search",
      role: "tool_activity",
      toolName: "search",
      activityOrder: 4,
    },
    {
      id: "tool-extension-a",
      role: "tool_activity",
      toolName: "extension_search",
      activityOrder: 2,
    },
    {
      id: "tool-extension-b",
      role: "tool_activity",
      toolName: "extension_search",
      activityOrder: 3,
    },
  ]);

  assert.deepEqual(
    grouped[0].activity.map((item) => item.id),
    ["tool-extension-a", "tool-extension-b", "tool-search"],
  );
});

test("groupMessages: unnumbered live activity keeps sequence against ordered projection activity", () => {
  const grouped = groupMessages([
    {
      id: "tool-web",
      role: "tool_activity",
      toolName: "search",
    },
    {
      id: "tool-extension-a",
      role: "tool_activity",
      toolName: "extension_search",
      activityOrder: 2,
    },
    {
      id: "tool-extension-b",
      role: "tool_activity",
      toolName: "extension_search",
      activityOrder: 3,
    },
  ]);

  assert.deepEqual(
    grouped[0].activity.map((item) => item.id),
    ["tool-web", "tool-extension-a", "tool-extension-b"],
  );
});

test("groupMessages: non-auxiliary messages break tool runs", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity" },
    { id: "m", role: "system", content: "paused" },
    { id: "b", role: "tool_activity" },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "activity-run");
  assert.equal(grouped[1].type, "message");
  assert.equal(grouped[2].type, "activity-run");
});

test("groupMessages: toolCalls-bearing messages stay inside activity runs", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity" },
    { id: "g", role: "assistant", toolCalls: [{ toolName: "read" }] },
    { id: "b", role: "tool_activity" },
  ]);

  assert.equal(grouped.length, 1);
  assert.equal(grouped[0].type, "activity-run");
  assert.deepEqual(grouped[0].activity.map((item) => item.id), ["a", "g", "b"]);
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

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "message");
  assert.equal(grouped[0].message.id, "u");
  assert.equal(grouped[1].type, "activity-run");
  assert.deepEqual(grouped[1].activity.map((item) => item.id), ["a", "r"]);
  assert.equal(grouped[2].type, "message");
  assert.equal(grouped[2].message.id, "m");
});

test("groupMessages: trailing activity stays separate from earlier tool runs", () => {
  const grouped = groupMessages([
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "m", role: "assistant", content: "Done.", isFinalReply: true },
    { id: "b", role: "tool_activity", toolName: "grep" },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "activity-run");
  assert.deepEqual(grouped[0].activity.map((item) => item.id), ["a"]);
  assert.equal(grouped[1].type, "activity-run");
  assert.deepEqual(grouped[1].activity.map((item) => item.id), ["b"]);
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
  assert.equal(grouped[1].type, "activity-run");
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
  assert.equal(grouped[1].type, "activity-run");
  assert.deepEqual(grouped[1].activity.map((item) => item.id), ["a", "b"]);
});

test("groupMessages: follow-up user does not break prior final reply ordering", () => {
  const grouped = groupMessages([
    { id: "m", role: "assistant", content: "answer", isFinalReply: true },
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "r", role: "thinking", content: "checking" },
    { id: "u", role: "user", content: "did you finish?" },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "activity-run");
  assert.deepEqual(grouped[0].activity.map((item) => item.id), ["a", "r"]);
  assert.equal(grouped[1].type, "message");
  assert.equal(grouped[1].message.id, "m");
  assert.equal(grouped[2].type, "message");
  assert.equal(grouped[2].message.id, "u");
});

test("groupMessages: delayed same-run activity moves before its final reply", () => {
  const grouped = groupMessages([
    { id: "u1", role: "user", content: "check email", turnRunId: "run-1" },
    {
      id: "m1",
      role: "assistant",
      content: "Here are the top emails.",
      isFinalReply: true,
      turnRunId: "run-1",
    },
    { id: "u2", role: "user", content: "now check calendar", turnRunId: "run-2" },
    {
      id: "thinking-thinking:run-1:3",
      role: "thinking",
      content: "rank emails",
      turnRunId: "run-1",
    },
    {
      id: "tool-gmail",
      role: "tool_activity",
      toolName: "gmail",
      turnRunId: "run-1",
    },
  ]);

  assert.equal(grouped.length, 4);
  assert.deepEqual(
    grouped.map((item) => item.type === "activity-run" ? item.id : item.message.id),
    ["u1", "activity-run-thinking-thinking:run-1:3", "m1", "u2"],
  );
  assert.deepEqual(
    grouped[1].activity.map((item) => item.id),
    ["thinking-thinking:run-1:3", "tool-gmail"],
  );
});

test("groupMessages: delayed different-run activity stays with the later turn", () => {
  const grouped = groupMessages([
    {
      id: "m1",
      role: "assistant",
      content: "Done.",
      isFinalReply: true,
      turnRunId: "run-1",
    },
    { id: "u2", role: "user", content: "next", turnRunId: "run-2" },
    {
      id: "tool-calendar",
      role: "tool_activity",
      toolName: "calendar",
      turnRunId: "run-2",
    },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "message");
  assert.equal(grouped[0].message.id, "m1");
  assert.equal(grouped[1].type, "message");
  assert.equal(grouped[1].message.id, "u2");
  assert.equal(grouped[2].type, "activity-run");
  assert.deepEqual(grouped[2].activity.map((item) => item.id), ["tool-calendar"]);
});

test("groupMessages: system note after activity keeps original order", () => {
  const grouped = groupMessages([
    { id: "m", role: "assistant", content: "answer", isFinalReply: true },
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "s", role: "system", content: "later note" },
  ]);

  assert.equal(grouped.length, 3);
  assert.equal(grouped[0].type, "message");
  assert.equal(grouped[0].message.id, "m");
  assert.equal(grouped[1].type, "activity-run");
  assert.equal(grouped[2].type, "message");
  assert.equal(grouped[2].message.id, "s");
});

test("groupMessages: toolCalls assistant is hoisted as activity, not final reply", () => {
  const grouped = groupMessages([
    { id: "m", role: "assistant", content: "answer", isFinalReply: true },
    { id: "a", role: "tool_activity", toolName: "read" },
    { id: "g", role: "assistant", toolCalls: [{ toolName: "read" }] },
  ]);

  assert.equal(grouped.length, 2);
  assert.equal(grouped[0].type, "activity-run");
  assert.deepEqual(grouped[0].activity.map((item) => item.id), ["a", "g"]);
  assert.equal(grouped[1].type, "message");
  assert.equal(grouped[1].message.id, "m");
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
  assert.equal(grouped[1].type, "activity-run");
  assert.deepEqual(grouped[1].activity.map((item) => item.id), ["a", "b"]);
});

test("groupMessages: activity run uses projection order for tool cards", () => {
  const grouped = groupMessages([
    {
      id: "tool-nearai",
      role: "tool_activity",
      toolName: "web_search",
      activityOrder: 3,
    },
    {
      id: "tool-web",
      role: "tool_activity",
      toolName: "search",
      activityOrder: 1,
    },
    {
      id: "tool-install",
      role: "tool_activity",
      toolName: "extension_install",
      activityOrder: 2,
    },
  ]);

  assert.equal(grouped.length, 1);
  assert.deepEqual(
    grouped[0].activity.map((item) => item.id),
    ["tool-web", "tool-install", "tool-nearai"],
  );
});

test("groupMessages: activity run falls back to tool update timestamps", () => {
  const grouped = groupMessages([
    {
      id: "tool-nearai",
      role: "tool_activity",
      toolName: "web_search",
      updatedAt: "2026-06-16T09:44:43.599792Z",
    },
    {
      id: "tool-web",
      role: "tool_activity",
      toolName: "search",
      updatedAt: "2026-06-16T09:44:08.580375Z",
    },
    {
      id: "tool-list",
      role: "tool_activity",
      toolName: "list_dir",
      updatedAt: "2026-06-16T09:44:23.277989Z",
    },
  ]);

  assert.equal(grouped.length, 1);
  assert.deepEqual(
    grouped[0].activity.map((item) => item.id),
    ["tool-web", "tool-list", "tool-nearai"],
  );
});
