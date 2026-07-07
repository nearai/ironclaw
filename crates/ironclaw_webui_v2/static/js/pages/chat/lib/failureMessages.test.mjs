import assert from "node:assert/strict";
import test from "node:test";

import {
  CONNECTION_LOST_RUN_FAILURE_MESSAGE,
  failureMessageForRunStatus,
  rewriteConnectionLostRunFailures,
  upsertConnectionLostRunFailure,
} from "./failureMessages.js";
import { CONNECTION_STATUS } from "./connection-status.js";

test("failureMessageForRunStatus prefers trimmed failureSummary", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_failed",
      failureSummary: "  The driver stopped unexpectedly.  ",
    }),
    "The driver stopped unexpectedly.",
  );
});

test("failureMessageForRunStatus formats category underscores", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_invalid_request",
      failureSummary: null,
    }),
    "The run failed: driver invalid request.",
  );
});

test("failureMessageForRunStatus uses recovery_required fallback", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "recovery_required",
      failureCategory: null,
      failureSummary: null,
    }),
    "The run is awaiting recovery — backend reported `recovery_required`.",
  );
});

test("failureMessageForRunStatus handles whitespace-only summary", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "lease_expired",
      failureSummary: "   ",
    }),
    "The run failed: lease expired.",
  );
});

test("failureMessageForRunStatus prefers connection copy for disconnected driver_unavailable failures", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
      connectionStatus: CONNECTION_STATUS.DISCONNECTED,
    }),
    CONNECTION_LOST_RUN_FAILURE_MESSAGE,
  );
});

test("failureMessageForRunStatus keeps non-connection driver failures unchanged", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
      connectionStatus: CONNECTION_STATUS.CONNECTED,
    }),
    "The run failed because the execution driver was temporarily unavailable.",
  );
});

test("failureMessageForRunStatus does not hide unrelated failures while disconnected", () => {
  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: "driver_invalid_request",
      failureSummary:
        "The run failed because the execution driver rejected the request.",
      connectionStatus: CONNECTION_STATUS.DISCONNECTED,
    }),
    "The run failed because the execution driver rejected the request.",
  );
});

test("rewriteConnectionLostRunFailures updates existing driver_unavailable bubbles after a disconnect", () => {
  const messages = [
    {
      id: "err-run-1",
      role: "error",
      content:
        "The run failed because the execution driver was temporarily unavailable.",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
    },
    {
      id: "err-run-2",
      role: "error",
      content:
        "The run failed because the execution driver rejected the request.",
      failureCategory: "driver_invalid_request",
      failureSummary:
        "The run failed because the execution driver rejected the request.",
    },
  ];

  const next = rewriteConnectionLostRunFailures(messages, { runId: "run-1" });

  assert.notEqual(next, messages);
  assert.equal(next[0].content, CONNECTION_LOST_RUN_FAILURE_MESSAGE);
  assert.equal(
    next[1].content,
    "The run failed because the execution driver rejected the request.",
  );
  assert.equal(rewriteConnectionLostRunFailures(next, { runId: "run-1" }), next);
});

test("rewriteConnectionLostRunFailures leaves history alone without a run id", () => {
  const messages = [
    {
      id: "err-old-run",
      role: "error",
      content:
        "The run failed because the execution driver was temporarily unavailable.",
      failureCategory: "driver_unavailable",
      failureSummary:
        "The run failed because the execution driver was temporarily unavailable.",
    },
  ];

  assert.equal(rewriteConnectionLostRunFailures(messages, { runId: null }), messages);
  assert.equal(rewriteConnectionLostRunFailures(messages, {}), messages);
});

test("upsertConnectionLostRunFailure appends scoped connection error", () => {
  const messages = [
    {
      id: "err-old-run",
      role: "error",
      content:
        "The run failed because the execution driver was temporarily unavailable.",
      failureCategory: "driver_unavailable",
    },
  ];

  const next = upsertConnectionLostRunFailure(messages, {
    runId: "run-1",
    timestamp: "2026-06-02T00:00:00.000Z",
  });

  assert.equal(messages.length, 1);
  assert.equal(next.length, 2);
  assert.equal(next[0].content, messages[0].content);
  assert.deepEqual(next[1], {
    id: "err-run-1",
    role: "error",
    content: CONNECTION_LOST_RUN_FAILURE_MESSAGE,
    timestamp: "2026-06-02T00:00:00.000Z",
    failureStatus: "failed",
    failureCategory: "connection_lost",
    failureSummary: CONNECTION_LOST_RUN_FAILURE_MESSAGE,
  });
});

test("upsertConnectionLostRunFailure does not rewrite history when run id is unknown", () => {
  const historicalFailure =
    "The run failed because the execution driver was temporarily unavailable.";
  const messages = [
    {
      id: "err-old-run",
      role: "error",
      content: historicalFailure,
      failureCategory: "driver_unavailable",
    },
  ];

  const next = upsertConnectionLostRunFailure(messages, {
    timestamp: "2026-06-02T00:00:00.000Z",
  });

  assert.equal(next.length, 2);
  assert.equal(next[0].content, historicalFailure);
  assert.equal(next[1].id, "err-connection-lost");
  assert.equal(next[1].content, CONNECTION_LOST_RUN_FAILURE_MESSAGE);
});
