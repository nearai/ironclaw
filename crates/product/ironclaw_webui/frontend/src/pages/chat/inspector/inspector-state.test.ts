import assert from "node:assert/strict";
import { test } from "vitest";

import {
  INSPECTOR_HEALTH,
  INSPECTOR_PREFERENCES_KEY,
  healthForInspectorStatus,
  inspectorDebugEnabled,
  inspectorViewportMode,
  latestInspectorRunId,
  readInspectorPreferences,
  shouldAcceptInspectorCursor,
  writeInspectorPreferences,
} from "./inspector-state";

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
