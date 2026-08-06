// @vitest-environment jsdom

import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "vitest";

import {
  publishProductInspectorActivity,
  subscribeProductInspectorActivity,
} from "./product-activity";

beforeEach(() => window.history.replaceState({}, "", "/?debug=true"));
afterEach(() => window.history.replaceState({}, "", "/"));

test("product activity is scoped and deduplicates replayed lifecycle states", () => {
  const observed: string[] = [];
  const unsubscribe = subscribeProductInspectorActivity(
    "thread-product-activity",
    "run-product-activity",
    (activity) => observed.push(`${activity.kind}:${activity.activityId}`),
  );
  const started = {
    threadId: "thread-product-activity",
    runId: "run-product-activity",
    kind: "tool_started",
    activityId: "activity-a",
    summary: "Tool invocation started",
    dedupeKey: "tool:activity-a:started",
  };
  publishProductInspectorActivity(started);
  publishProductInspectorActivity(started);
  publishProductInspectorActivity({
    ...started,
    kind: "tool_completed",
    summary: "Tool invocation completed",
    dedupeKey: "tool:activity-a:completed",
  });
  publishProductInspectorActivity({
    ...started,
    threadId: "thread-other",
    dedupeKey: "tool:activity-a:other-thread",
  });
  unsubscribe();

  assert.deepEqual(observed, [
    "tool_started:activity-a",
    "tool_completed:activity-a",
  ]);
});

test("product activity is inert when inspector mode is disabled", () => {
  window.history.replaceState({}, "", "/");
  const observed: string[] = [];
  const unsubscribe = subscribeProductInspectorActivity(
    "thread-disabled",
    "run-disabled",
    (activity) => observed.push(activity.kind),
  );

  publishProductInspectorActivity({
    threadId: "thread-disabled",
    runId: "run-disabled",
    kind: "progress",
    summary: "Run progress",
    dedupeKey: "progress:disabled",
  });
  unsubscribe();

  assert.deepEqual(observed, []);
});
