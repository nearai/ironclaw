import assert from "node:assert/strict";
import { test } from "vitest";

import { setAuthScope } from "../../../lib/auth-scope";
import { ActivityKind } from "./activity-kind";
import {
  MAX_INSPECTOR_ACTIVITY_ENTRIES,
  MAX_INSPECTOR_RUNS_PER_THREAD,
  inspectorRunHistoryKey,
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
  inspectorStreamSessionKey,
  readInspectorStreamCursor,
  readInspectorStreamMetrics,
  recordInspectorDiagnosticUpdate,
  recordInspectorReconnect,
} from "./inspector-stream-session";
import {
  INSPECTOR_DEBUG_ENABLED_KEY,
  inspectorDebugEnabled,
  latestInspectorRunId,
  persistInspectorDebugPreference,
} from "./inspector-shell";

function storage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    dump: () => Object.fromEntries(values),
  };
}

test("debug activation follows explicit query values and persists across routes", () => {
  const memory = storage();
  assert.equal(inspectorDebugEnabled("", memory), false);

  assert.equal(inspectorDebugEnabled("?debug=true", memory), true);
  persistInspectorDebugPreference("?debug=true", memory);
  assert.equal(memory.dump()[INSPECTOR_DEBUG_ENABLED_KEY], "true");
  assert.equal(inspectorDebugEnabled("", memory), true);

  assert.equal(inspectorDebugEnabled("?debug=false", memory), false);
  persistInspectorDebugPreference("?debug=false", memory);
  assert.equal(memory.dump()[INSPECTOR_DEBUG_ENABLED_KEY], "false");
  assert.equal(inspectorDebugEnabled("", memory), false);

  assert.equal(inspectorDebugEnabled("?debug=1", memory), false);
  assert.equal(inspectorDebugEnabled("?foo=1&debug=true", memory), true);
});

test("debug activation fails closed when query parsing throws", () => {
  const OriginalURLSearchParams = globalThis.URLSearchParams;
  globalThis.URLSearchParams = class URLSearchParamsFailure {
    constructor() {
      throw new TypeError("query parsing unavailable");
    }
  } as typeof URLSearchParams;
  try {
    assert.equal(inspectorDebugEnabled("?debug=true"), false);
  } finally {
    globalThis.URLSearchParams = OriginalURLSearchParams;
  }
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

  assert.equal(rows.filter((row) => row.kind === ActivityKind.TurnStarted).length, 1);
  assert.equal(rows.filter((row) => row.kind === ActivityKind.StreamDisconnected).length, 2);
  assert.equal(rows.find((row) => row.kind === ActivityKind.TurnStarted)?.sequence, 1);
});

test("activity reducer gives mixed server and local rows one transitive order", () => {
  const rows = reduceInspectorActivity(
    {
      stream_id: "stream-mixed",
      activity: [
        { sequence: 1, event: activity("model_call_started", {
          occurred_at: "2026-08-06T10:00:00Z",
          model_call_id: "call-1",
        }) },
        { sequence: 2, event: activity("model_call_completed", {
          occurred_at: "2026-08-06T08:00:00Z",
          model_call_id: "call-1",
        }) },
      ],
    },
    [
      {
        local_id: "local-before-next-sequence",
        update: { type: "activity", data: activity("stream_disconnected", {
          occurred_at: "2026-08-06T09:00:00Z",
        }) },
      },
      {
        stream_id: "stream-mixed",
        sequence: 3,
        update: { type: "activity", data: activity("progress", {
          occurred_at: "2026-08-06T07:00:00Z",
        }) },
      },
      {
        local_id: "local-after-next-sequence",
        update: { type: "activity", data: activity("stream_resumed", {
          occurred_at: "2026-08-06T06:00:00Z",
        }) },
      },
    ],
  );

  assert.deepEqual(
    rows.map((row) => row.key),
    [
      "stream-mixed:1",
      "stream-mixed:2",
      "local:local-before-next-sequence",
      "stream-mixed:3",
      "local:local-after-next-sequence",
    ],
  );
  assert.equal(rows[0].pending, false);
});

test("activity reducer retains prompt preparation for every loop iteration", () => {
  const rows = reduceInspectorActivity(
    {
      stream_id: "stream-prompts",
      activity: [
        { sequence: 1, event: activity("prompt_prepared") },
        { sequence: 2, event: activity("model_call_started", { model_call_id: "call-1" }) },
        { sequence: 3, event: activity("model_call_completed", { model_call_id: "call-1" }) },
        { sequence: 4, event: activity("prompt_prepared") },
        { sequence: 5, event: activity("model_call_started", { model_call_id: "call-2" }) },
      ],
    },
    [],
  );

  assert.deepEqual(
    rows.filter((row) => row.kind === ActivityKind.PromptPrepared).map((row) => row.sequence),
    [1, 4],
  );
});

test("run navigation history is thread-scoped, deduplicated, and bounded", () => {
  const memory = storage();
  assert.deepEqual(rememberInspectorRun("thread-a", "run-1", memory), ["run-1"]);
  assert.deepEqual(rememberInspectorRun("thread-a", "run-2", memory), ["run-1", "run-2"]);
  assert.deepEqual(rememberInspectorRun("thread-a", "run-1", memory), ["run-2", "run-1"]);
  assert.deepEqual(rememberInspectorRun("thread-b", "run-b", memory), ["run-b"]);
  const saved = JSON.parse(memory.dump()[inspectorRunHistoryKey()]);
  assert.deepEqual(saved["thread-a"], ["run-2", "run-1"]);
  assert.deepEqual(saved["thread-b"], ["run-b"]);

  // Bounded to the host's retention depth: navigation must not offer a turn
  // whose diagnostics the host has already evicted.
  for (let index = 1; index <= MAX_INSPECTOR_RUNS_PER_THREAD + 1; index += 1) {
    rememberInspectorRun("bounded-thread", `run-${index}`, memory);
  }
  const bounded = JSON.parse(memory.dump()[inspectorRunHistoryKey()])["bounded-thread"];
  assert.equal(bounded.length, MAX_INSPECTOR_RUNS_PER_THREAD);
  assert.equal(bounded[0], "run-2");
  assert.equal(bounded.at(-1), `run-${MAX_INSPECTOR_RUNS_PER_THREAD + 1}`);
});

test("stream metrics persist in the browser session without duplicate update counts", () => {
  const memory = storage();
  assert.deepEqual(readInspectorStreamMetrics(memory), {
    reconnectCount: 0,
    receivedUpdateCount: 0,
    lastUpdateAt: null,
  });

  assert.deepEqual(recordInspectorReconnect(memory), {
    reconnectCount: 1,
    receivedUpdateCount: 0,
    lastUpdateAt: null,
  });
  const first = recordInspectorDiagnosticUpdate(
    "thread-a/run-a",
    "stream-a:1",
    "2026-08-06T10:00:00.000Z",
    memory,
  );
  assert.equal(first.accepted, true);
  assert.equal(first.metrics.receivedUpdateCount, 1);

  const duplicate = recordInspectorDiagnosticUpdate(
    "thread-a/run-a",
    "stream-a:1",
    "2026-08-06T10:01:00.000Z",
    memory,
  );
  assert.equal(duplicate.accepted, false);
  assert.equal(duplicate.metrics.receivedUpdateCount, 1);
  assert.equal(duplicate.metrics.lastUpdateAt, "2026-08-06T10:00:00.000Z");
  assert.equal(readInspectorStreamCursor("thread-a/run-a", memory), "stream-a:1");
  assert.ok(memory.dump()[inspectorStreamSessionKey()]);

  for (let index = 0; index < 40; index += 1) {
    recordInspectorDiagnosticUpdate(
      `thread-${index}/run-${index}`,
      `stream-${index}:1`,
      "2026-08-06T10:02:00.000Z",
      memory,
    );
  }
  const bounded = JSON.parse(memory.dump()[inspectorStreamSessionKey()]);
  assert.equal(bounded.scopeOrder.length, 32);
  assert.equal(Object.keys(bounded.cursors).length, 32);
  assert.equal(bounded.scopeOrder[0], "thread-8/run-8");
});

test("browser-session inspector state is namespaced by the authenticated caller", () => {
  const memory = storage();
  try {
    setAuthScope({ tenant_id: "tenant-a", user_id: "user-a" });
    rememberInspectorRun("thread-a", "run-a", memory);
    recordInspectorDiagnosticUpdate(
      "thread-a/run-a",
      "stream-a:7",
      "2026-08-08T10:00:00.000Z",
      memory,
    );
    const ownerHistoryKey = inspectorRunHistoryKey();
    const ownerStreamKey = inspectorStreamSessionKey();
    assert.equal(readInspectorStreamCursor("thread-a/run-a", memory), "stream-a:7");
    assert.equal(readInspectorStreamMetrics(memory).receivedUpdateCount, 1);

    // A bearer session change inside one tab must not inherit the previous
    // caller's observed runs, resume cursors, or observation counters.
    setAuthScope({ tenant_id: "tenant-a", user_id: "user-b" });
    assert.notEqual(inspectorRunHistoryKey(), ownerHistoryKey);
    assert.notEqual(inspectorStreamSessionKey(), ownerStreamKey);
    assert.deepEqual(rememberInspectorRun("thread-a", null, memory), []);
    assert.equal(readInspectorStreamCursor("thread-a/run-a", memory), null);
    assert.deepEqual(readInspectorStreamMetrics(memory), {
      reconnectCount: 0,
      receivedUpdateCount: 0,
      lastUpdateAt: null,
    });

    // Returning to the first caller still finds that caller's own state.
    setAuthScope({ tenant_id: "tenant-a", user_id: "user-a" });
    assert.deepEqual(rememberInspectorRun("thread-a", null, memory), ["run-a"]);
    assert.equal(readInspectorStreamCursor("thread-a/run-a", memory), "stream-a:7");
  } finally {
    setAuthScope(null);
  }
});
