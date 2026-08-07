// @vitest-environment jsdom

import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "vitest";

import { publishProductInspectorEnvelope } from "./product-activity-envelope";
import { subscribeProductInspectorActivity } from "./product-activity";

beforeEach(() => window.history.replaceState({}, "", "/?debug=true"));
afterEach(() => window.history.replaceState({}, "", "/"));

function untaggedActivity() {
  return {
    invocation_id: "projection-invocation",
    status: "started",
  };
}

test("projection activity uses the batch's sole run id", () => {
  const observed = [];
  const unsubscribe = subscribeProductInspectorActivity(
    "thread-projection-sole-run",
    "run-b",
    (activity) => observed.push(activity),
  );

  publishProductInspectorEnvelope(
    {
      type: "projection_snapshot",
      frame: {
        state: {
          items: [
            { run_status: { run_id: "run-b", status: "running" } },
            { capability_activity: untaggedActivity() },
          ],
        },
      },
    },
    "thread-projection-sole-run",
    "run-stale",
  );
  unsubscribe();

  assert.equal(observed.find((activity) =>
    activity.activityId === "projection-invocation")?.runId, "run-b");
});

test("projection activity stays unscoped when the batch is ambiguous", () => {
  const observed = [];
  const unsubscribeActive = subscribeProductInspectorActivity(
    "thread-projection-ambiguous",
    "run-active",
    (activity) => observed.push(activity),
  );
  const unsubscribeOther = subscribeProductInspectorActivity(
    "thread-projection-ambiguous",
    "run-other",
    (activity) => observed.push(activity),
  );

  publishProductInspectorEnvelope(
    {
      type: "projection_update",
      frame: {
        state: {
          items: [
            { run_status: { run_id: "run-active", status: "running" } },
            { run_status: { run_id: "run-other", status: "running" } },
            { capability_activity: untaggedActivity() },
          ],
        },
      },
    },
    "thread-projection-ambiguous",
    "run-active",
  );
  unsubscribeActive();
  unsubscribeOther();

  assert.equal(observed.some((activity) =>
    activity.activityId === "projection-invocation"), false);
});
