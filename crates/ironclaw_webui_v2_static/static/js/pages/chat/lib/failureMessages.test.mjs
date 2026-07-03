import assert from "node:assert/strict";
import test from "node:test";

import { failureMessageForRunStatus } from "./failureMessages.js";

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
