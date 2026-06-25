import assert from "node:assert/strict";
import test from "node:test";

import {
  buildScopedLogsPath,
  normalizeLogEntry,
  normalizeOperatorLogsResponse,
} from "./logs-data.js";

test("normalizeOperatorLogsResponse reads command-plane nested logs payload", () => {
  const response = normalizeOperatorLogsResponse({
    status: "available",
    logs: {
      source: "in_memory_tracing",
      next_cursor: "before:4",
      tail_supported: true,
      follow_supported: false,
      entries: [
        {
          id: "5",
          timestamp: "2026-06-11T12:00:00.123Z",
          level: "warn",
          target: "ironclaw::test",
          message: "something happened",
          thread_id: "thread-a",
          run_id: "run-a",
          turn_id: "turn-a",
          tool_call_id: "tool-a",
          tool_name: "shell",
          source: "slack",
        },
      ],
    },
  });

  assert.equal(response.source, "in_memory_tracing");
  assert.equal(response.nextCursor, "before:4");
  assert.equal(response.tailSupported, true);
  assert.equal(response.followSupported, false);
  assert.deepEqual(response.entries, [
    {
      id: "5",
      timestamp: "2026-06-11T12:00:00.123Z",
      level: "warn",
      target: "ironclaw::test",
      message: "something happened",
      threadId: "thread-a",
      runId: "run-a",
      turnId: "turn-a",
      toolCallId: "tool-a",
      toolName: "shell",
      source: "slack",
    },
  ]);
});

test("normalizeOperatorLogsResponse tolerates direct log payloads", () => {
  const response = normalizeOperatorLogsResponse({
    entries: [{ id: 7, level: "ERROR", message: "failed" }],
  });

  assert.deepEqual(response.entries, [
    {
      id: "7",
      timestamp: "",
      level: "error",
      target: "",
      message: "failed",
      threadId: null,
      runId: null,
      turnId: null,
      toolCallId: null,
      toolName: null,
      source: null,
    },
  ]);
});

test("normalizeLogEntry creates a stable fallback id", () => {
  assert.equal(
    normalizeLogEntry({
      timestamp: "2026-06-11T12:00:00Z",
      target: "ironclaw::test",
      message: "hello",
    }).id,
    "2026-06-11T12:00:00Z:ironclaw::test:hello",
  );
});

test("buildScopedLogsPath encodes structured log filters", () => {
  assert.equal(
    buildScopedLogsPath({
      threadId: "thread a",
      runId: "run-a",
      toolCallId: "tool/a",
      source: "slack",
    }),
    "/logs?thread_id=thread+a&run_id=run-a&tool_call_id=tool%2Fa&source=slack",
  );
});

test("buildScopedLogsPath returns a basename-relative path", () => {
  // The result is handed to a react-router <Link to>/navigate(), which prepends
  // the router basename ("/v2"). A "/v2"-prefixed return value would resolve to
  // the doubled "/v2/v2/logs"; guard that it never happens for any input.
  for (const args of [
    {},
    { threadId: "t" },
    { threadId: "t", runId: "r" },
    { toolName: "shell", source: "slack" },
  ]) {
    const path = buildScopedLogsPath(args);
    assert.ok(
      path.startsWith("/logs"),
      `expected basename-relative /logs path, got ${path}`,
    );
    assert.ok(!path.startsWith("/v2"), `must not include basename, got ${path}`);
  }
});
