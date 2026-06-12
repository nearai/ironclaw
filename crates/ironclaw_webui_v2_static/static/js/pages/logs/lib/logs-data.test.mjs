import assert from "node:assert/strict";
import test from "node:test";

import {
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
