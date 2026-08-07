import assert from "node:assert/strict";
import { test } from "vitest";

import {
  INSPECTOR_RUN_HISTORY_KEY,
  MAX_INSPECTOR_ACTIVITY_ENTRIES,
  reduceInspectorActivity,
  rememberInspectorRun,
} from "./inspector-activity";
import {
  INSPECTOR_HEALTH,
  INSPECTOR_PREFERENCES_KEY,
  healthForInspectorStatus,
  inspectorViewportMode,
  readInspectorPreferences,
  shouldAcceptInspectorCursor,
  writeInspectorPreferences,
} from "./inspector-state";
import {
  inspectorDebugEnabled,
  latestInspectorRunId,
} from "./inspector-shell";

function storage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    dump: () => Object.fromEntries(values),
  };
}

test("debug activation accepts only the explicit true query value", () => {
  assert.equal(inspectorDebugEnabled("?debug=true"), true);
  assert.equal(inspectorDebugEnabled("?foo=1&debug=true"), true);
  assert.equal(inspectorDebugEnabled("?debug=false"), false);
  assert.equal(inspectorDebugEnabled("?debug=1"), false);
  assert.equal(inspectorDebugEnabled(""), false);
});

test("preferences are session-scoped, validated, and round-trip", () => {
  const memory = storage();
  assert.deepEqual(readInspectorPreferences(memory), {
    open: true,
    activeTab: "prompt",
  });
  writeInspectorPreferences({ open: false, activeTab: "stats" }, memory);
  assert.deepEqual(readInspectorPreferences(memory), {
    open: false,
    activeTab: "stats",
  });
  assert.equal(
    memory.dump()[INSPECTOR_PREFERENCES_KEY],
    JSON.stringify({ open: false, activeTab: "stats" }),
  );

  const invalid = storage({
    [INSPECTOR_PREFERENCES_KEY]: JSON.stringify({ open: "yes", activeTab: "unknown" }),
  });
  assert.deepEqual(readInspectorPreferences(invalid), {
    open: true,
    activeTab: "prompt",
  });
});

test("cursor acceptance deduplicates and rejects backwards updates", () => {
  const stream = "550e8400-e29b-41d4-a716-446655440000";
  assert.equal(shouldAcceptInspectorCursor(null, `${stream}:1`), true);
  assert.equal(shouldAcceptInspectorCursor(`${stream}:1`, `${stream}:1`), false);
  assert.equal(shouldAcceptInspectorCursor(`${stream}:2`, `${stream}:1`), false);
  assert.equal(shouldAcceptInspectorCursor(`${stream}:1`, `${stream}:2`), true);
  assert.equal(shouldAcceptInspectorCursor(`${stream}:2`, "new-stream:1"), true);
  assert.equal(shouldAcceptInspectorCursor(`${stream}:2`, "bad"), false);
});

test("viewport modes keep unsupported mobile hidden", () => {
  assert.equal(inspectorViewportMode(375), "mobile");
  assert.equal(inspectorViewportMode(640), "overlay");
  assert.equal(inspectorViewportMode(1024), "overlay");
  assert.equal(inspectorViewportMode(1280), "sidebar");
});

test("latest run remains inspectable after the active run settles", () => {
  assert.equal(
    latestInspectorRunId({ runId: "run-live" }, [{ turnRunId: "run-old" }]),
    "run-live",
  );
  assert.equal(
    latestInspectorRunId(null, [
      { turnRunId: "run-old" },
      { content: "progress" },
      { turnRunId: "run-latest" },
    ]),
    "run-latest",
  );
  assert.equal(latestInspectorRunId(null, []), null);
});

test("HTTP status classification distinguishes auth, absence, and retry", () => {
  assert.equal(healthForInspectorStatus(403), INSPECTOR_HEALTH.FORBIDDEN);
  assert.equal(healthForInspectorStatus(404), INSPECTOR_HEALTH.UNAVAILABLE);
  assert.equal(healthForInspectorStatus(503), INSPECTOR_HEALTH.RECONNECTING);
  assert.equal(healthForInspectorStatus(400), INSPECTOR_HEALTH.DISCONNECTED);
});

function activity(kind: string, options: Record<string, unknown> = {}) {
  return {
    occurred_at: options.occurred_at || "2026-08-06T10:00:00Z",
    kind,
    iteration: options.iteration ?? null,
    activity_id: options.activity_id ?? null,
    model_call_id: options.model_call_id ?? null,
    summary: options.summary ?? null,
  };
}

test("activity reducer orders, deduplicates, and settles correlated model calls", () => {
  const snapshot = {
    stream_id: "stream-a",
    activity: [
      { sequence: 3, event: activity("model_call_completed", { model_call_id: "call-a" }) },
      { sequence: 1, event: activity("turn_started") },
      { sequence: 2, event: activity("model_call_started", { model_call_id: "call-a" }) },
    ],
  };
  const rows = reduceInspectorActivity(snapshot, [
    {
      stream_id: "stream-a",
      sequence: 3,
      update: { type: "activity", data: activity("model_call_completed", { model_call_id: "call-a" }) },
    },
    {
      stream_id: "stream-a",
      sequence: 4,
      update: { type: "activity", data: activity("model_call_started", { model_call_id: "call-b" }) },
    },
  ]);
  assert.deepEqual(rows.map((row) => row.sequence), [1, 2, 3, 4]);
  assert.equal(rows[1].pending, false);
  assert.equal(rows[3].pending, true);
});

test("activity reducer bounds retention and keeps transport events", () => {
  const activityEntries = Array.from(
    { length: MAX_INSPECTOR_ACTIVITY_ENTRIES + 5 },
    (_, index) => ({ sequence: index + 1, event: activity("progress") }),
  );
  const rows = reduceInspectorActivity(
    { stream_id: "stream-a", activity: activityEntries },
    [{
      local_id: "transport-1",
      update: { type: "activity", data: activity("stream_resumed", { occurred_at: "2026-08-06T11:00:00Z" }) },
    }],
  );
  assert.equal(rows.length, MAX_INSPECTOR_ACTIVITY_ENTRIES);
  assert.equal(rows.at(-1)?.kind, "stream_resumed");
  assert.equal(rows[0].sequence, 7);
});

test("activity reducer replaces local lifecycle hints with authoritative diagnostics", () => {
  const rows = reduceInspectorActivity(
    {
      stream_id: "stream-authoritative",
      activity: [{ sequence: 1, event: activity("turn_started") }],
    },
    [
      {
        local_id: "product-turn",
        update: { type: "activity", data: activity("turn_started") },
      },
      {
        local_id: "disconnect-1",
        update: { type: "activity", data: activity("stream_disconnected") },
      },
      {
        local_id: "disconnect-2",
        update: { type: "activity", data: activity("stream_disconnected") },
      },
    ],
  );

  assert.equal(rows.filter((row) => row.kind === "turn_started").length, 1);
  assert.equal(rows.filter((row) => row.kind === "stream_disconnected").length, 2);
  assert.equal(rows.find((row) => row.kind === "turn_started")?.sequence, 1);
});

test("run navigation history is thread-scoped, deduplicated, and bounded", () => {
  const memory = storage();
  assert.deepEqual(rememberInspectorRun("thread-a", "run-1", memory), ["run-1"]);
  assert.deepEqual(rememberInspectorRun("thread-a", "run-2", memory), ["run-1", "run-2"]);
  assert.deepEqual(rememberInspectorRun("thread-a", "run-1", memory), ["run-2", "run-1"]);
  assert.deepEqual(rememberInspectorRun("thread-b", "run-b", memory), ["run-b"]);
  const saved = JSON.parse(memory.dump()[INSPECTOR_RUN_HISTORY_KEY]);
  assert.deepEqual(saved["thread-a"], ["run-2", "run-1"]);
  assert.deepEqual(saved["thread-b"], ["run-b"]);
});
