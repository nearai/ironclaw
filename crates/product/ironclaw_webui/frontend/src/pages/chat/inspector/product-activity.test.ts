// @vitest-environment jsdom

import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "vitest";

import { ActivityKind } from "./activity-kind";
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
    kind: ActivityKind.ToolStarted,
    activityId: "activity-a",
    summary: "Tool invocation started",
    dedupeKey: "tool:activity-a:started",
  };
  publishProductInspectorActivity(started);
  publishProductInspectorActivity(started);
  publishProductInspectorActivity({
    ...started,
    kind: ActivityKind.ToolCompleted,
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
    kind: ActivityKind.Progress,
    summary: "Run progress",
    dedupeKey: "progress:disabled",
  });
  unsubscribe();

  assert.deepEqual(observed, []);
});

test("product activity fails closed when debug query parsing throws", () => {
  const OriginalURLSearchParams = globalThis.URLSearchParams;
  globalThis.URLSearchParams = class URLSearchParamsFailure {
    constructor() {
      throw new TypeError("query parsing unavailable");
    }
  } as typeof URLSearchParams;
  const observed: string[] = [];
  const unsubscribe = subscribeProductInspectorActivity(
    "thread-query-failure",
    "run-query-failure",
    (activity) => observed.push(activity.kind),
  );
  try {
    publishProductInspectorActivity({
      threadId: "thread-query-failure",
      runId: "run-query-failure",
      kind: ActivityKind.Progress,
      summary: "Run progress",
      dedupeKey: "progress:query-failure",
    });
  } finally {
    unsubscribe();
    globalThis.URLSearchParams = OriginalURLSearchParams;
  }

  assert.deepEqual(observed, []);
});
