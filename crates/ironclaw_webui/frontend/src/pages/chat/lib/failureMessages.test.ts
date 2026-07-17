import assert from "node:assert/strict";
import { test } from "vitest";

import {
  CONNECTION_LOST_RUN_FAILURE_MESSAGE,
  failureMessageForRequestError,
  failureMessageForRunStatus,
  failureMessageForStreamError,
  rewriteConnectionLostRunFailures,
  upsertConnectionLostRunFailure,
} from "./failureMessages";
import { CONNECTION_STATUS } from "./connection-status";

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

test("failureMessageForRequestError prefers a safe throwable message", () => {
  assert.equal(
    failureMessageForRequestError(
      new Error("  AI provider account is out of credits.  "),
    ),
    "AI provider account is out of credits.",
  );
});

test("failureMessageForRequestError suppresses credential-bearing messages", () => {
  assert.equal(
    failureMessageForRequestError(
      new Error("Authorization: Bearer sk-proj-1234567890abcdef"),
    ),
    "The request failed before it could be sent.",
  );
  assert.equal(
    failureMessageForRequestError({
      message: "Provider rejected API key abcdef1234567890abcdef1234567890.",
    }),
    "The request failed before it could be sent.",
  );
});

test("failureMessageForRequestError has a stable fallback", () => {
  assert.equal(
    failureMessageForRequestError({ message: "   " }),
    "The request failed before it could be sent.",
  );
});

test("failureMessageForStreamError humanizes redacted stream tokens", () => {
  assert.equal(
    failureMessageForStreamError({
      error: "unavailable",
      kind: "service_unavailable",
      retryable: true,
    }),
    "The chat stream hit a retryable error: Service unavailable.",
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

test("failureMessageForRunStatus does not infer driver_unavailable from summary copy", () => {
  const summary =
    "The run failed because the execution driver was temporarily unavailable.";

  assert.equal(
    failureMessageForRunStatus({
      status: "failed",
      failureCategory: null,
      failureSummary: summary,
      connectionStatus: CONNECTION_STATUS.DISCONNECTED,
    }),
    summary,
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
