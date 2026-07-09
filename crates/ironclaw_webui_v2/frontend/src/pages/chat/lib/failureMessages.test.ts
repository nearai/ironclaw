import assert from "node:assert/strict";
import { test } from "vitest";

import {
  failureMessageForRequestError,
  failureMessageForRunStatus,
  failureMessageForStreamError,
} from "./failureMessages";

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

test("failureMessageForRequestError prefers the throwable message", () => {
  assert.equal(
    failureMessageForRequestError(
      new Error("  AI provider account is out of credits.  "),
    ),
    "AI provider account is out of credits.",
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
