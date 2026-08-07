// @vitest-environment jsdom

import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "vitest";

import { ActivityKind } from "./activity-kind";
import { reduceInspectorActivity } from "./inspector-activity";
import { publishProductInspectorEnvelope } from "./product-activity-envelope";
import {
  subscribeProductInspectorActivity,
  type ProductInspectorActivity,
} from "./product-activity";

beforeEach(() => window.history.replaceState({}, "", "/?debug=true"));
afterEach(() => window.history.replaceState({}, "", "/"));

function untaggedActivity() {
  return {
    invocation_id: "projection-invocation",
    status: "started",
  };
}

function reduceProductActivity(observed: ProductInspectorActivity[]) {
  return reduceInspectorActivity(null, observed.map((activity) => ({
    local_id: activity.localId,
    update: {
      type: "activity",
      data: {
        occurred_at: activity.occurredAt,
        kind: activity.kind,
        iteration: null,
        activity_id: activity.activityId,
        model_call_id: null,
        summary: {
          content: activity.summary,
          original_bytes: activity.summary.length,
          truncated: false,
        },
      },
    },
  })));
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

  assert.deepEqual(
    observed
      .filter((activity) => activity.activityId === null)
      .map((activity) => `${activity.runId}:${activity.kind}`),
    ["run-active:progress", "run-other:progress"],
  );
  assert.equal(observed.some((activity) =>
    activity.activityId === "projection-invocation"), false);
});

test("terminal tool and run failures settle their started activity", () => {
  const observed: ProductInspectorActivity[] = [];
  const unsubscribe = subscribeProductInspectorActivity(
    "thread-terminal-activity",
    "run-terminal-activity",
    (activity) => observed.push(activity),
  );

  publishProductInspectorEnvelope({
    type: "capability_activity",
    frame: {
      activity: {
        turn_run_id: "run-terminal-activity",
        invocation_id: "invocation-cancelled",
        status: "started",
      },
    },
  }, "thread-terminal-activity", null);
  publishProductInspectorEnvelope({
    type: "capability_activity",
    frame: {
      activity: {
        turn_run_id: "run-terminal-activity",
        invocation_id: "invocation-cancelled",
        status: "cancelled",
      },
    },
  }, "thread-terminal-activity", null);
  publishProductInspectorEnvelope({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-terminal-activity", status: "queued" } }],
      },
    },
  }, "thread-terminal-activity", null);
  publishProductInspectorEnvelope({
    type: "projection_update",
    frame: {
      state: {
        items: [{ run_status: { run_id: "run-terminal-activity", status: "failed" } }],
      },
    },
  }, "thread-terminal-activity", null);
  unsubscribe();

  const rows = reduceProductActivity(observed);
  assert.equal(
    rows.find((row) => row.kind === ActivityKind.ToolStarted)?.pending,
    false,
  );
  assert.equal(
    rows.find((row) => row.kind === ActivityKind.TurnStarted)?.pending,
    false,
  );
  assert.equal(
    rows.some((row) => row.kind === ActivityKind.ToolFailed),
    true,
  );
  assert.equal(
    rows.some((row) => row.kind === ActivityKind.FinalResponseCompleted),
    true,
  );
});
