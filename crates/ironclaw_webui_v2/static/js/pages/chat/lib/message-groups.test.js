import test from "node:test";
import assert from "node:assert/strict";

import { groupMessages } from "./message-groups.js";

test("tool activity ordering uses sequence when only one card has projection order", () => {
  const grouped = groupMessages([
    {
      id: "tool-second",
      role: "tool_activity",
      sequence: 2,
      activityOrder: 10,
    },
    {
      id: "tool-first",
      role: "tool_activity",
      sequence: 1,
      activityOrder: null,
    },
  ]);

  assert.equal(grouped.length, 1);
  assert.deepEqual(
    grouped[0].activity.map((message) => message.id),
    ["tool-first", "tool-second"],
  );
});

test("tool activity ordering uses projection order when both cards have it", () => {
  const grouped = groupMessages([
    {
      id: "tool-second",
      role: "tool_activity",
      sequence: 1,
      activityOrder: 20,
    },
    {
      id: "tool-first",
      role: "tool_activity",
      sequence: 2,
      activityOrder: 10,
    },
  ]);

  assert.equal(grouped.length, 1);
  assert.deepEqual(
    grouped[0].activity.map((message) => message.id),
    ["tool-first", "tool-second"],
  );
});
