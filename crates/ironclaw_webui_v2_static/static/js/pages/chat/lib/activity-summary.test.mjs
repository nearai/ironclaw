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

test("summarizeActivity: declined tools are neutral, not failed", () => {
  const summary = summarizeActivity([
    {
      id: "g",
      role: "assistant",
      toolCalls: [
        { id: "a", toolStatus: "success" },
        { id: "b", toolStatus: "declined" },
      ],
    },
  ]);

  assert.equal(summary.hasError, false);
  assert.equal(summary.hasDeclined, true);
  assert.equal(summary.label, "Activity - 2 tools, 1 declined");
});

test("summarizeActivity: summary label uses the active translator", () => {
  const zh = {
    "activity.summaryWithParts": "活动 - {parts}",
    "activity.reasoning": "{count} 条推理",
    "activity.tools": "{count} 个工具",
    "activity.failed": "{count} 个失败",
    "activity.separator": "、",
  };
  const t = (key, params = {}) =>
    (zh[key] || key).replace(/\{(\w+)\}/g, (_, name) => params[name] ?? `{${name}}`);

  const summary = summarizeActivity(
    [
      { id: "r", role: "thinking", content: "checking" },
      {
        id: "g",
        role: "assistant",
        toolCalls: [
          { id: "a", toolStatus: "error" },
          { id: "b", toolStatus: "success" },
        ],
      },
    ],
    t,
  );

  assert.equal(summary.label, "活动 - 1 条推理、2 个工具、1 个失败");
});
