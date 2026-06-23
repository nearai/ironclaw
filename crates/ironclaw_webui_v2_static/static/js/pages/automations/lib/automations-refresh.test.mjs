import assert from "node:assert/strict";
import test from "node:test";

import {
  AUTOMATIONS_BASE_REFETCH_MS,
  AUTOMATIONS_DUE_GRACE_MS,
  AUTOMATIONS_OVERDUE_REFETCH_MS,
  AUTOMATIONS_RUNNING_REFETCH_MS,
  nextAutomationsRefetchDelay,
} from "./automations-refresh.js";

test("nextAutomationsRefetchDelay returns null when no automation needs an early refresh", () => {
  assert.equal(nextAutomationsRefetchDelay([], 1_000), null);
  assert.equal(
    nextAutomationsRefetchDelay(
      [
        {
          state: "scheduled",
          next_run_timestamp: 1_000 + AUTOMATIONS_BASE_REFETCH_MS + 10_000,
        },
        {
          state: "paused",
          next_run_timestamp: 1_001,
        },
      ],
      1_000,
    ),
    null,
  );
});

test("nextAutomationsRefetchDelay refreshes shortly after the next schedule boundary", () => {
  assert.equal(
    nextAutomationsRefetchDelay(
      [
        {
          state: "scheduled",
          next_run_timestamp: 20_000,
        },
      ],
      10_000,
    ),
    10_000 + AUTOMATIONS_DUE_GRACE_MS,
  );
});

test("nextAutomationsRefetchDelay polls overdue schedulable automations quickly", () => {
  assert.equal(
    nextAutomationsRefetchDelay(
      [
        {
          state: "active",
          next_run_timestamp: 9_000,
        },
      ],
      10_000,
    ),
    AUTOMATIONS_OVERDUE_REFETCH_MS,
  );
});

test("nextAutomationsRefetchDelay follows running runs even when the automation is paused", () => {
  assert.equal(
    nextAutomationsRefetchDelay(
      [
        {
          state: "paused",
          has_running_run: true,
          next_run_timestamp: 9_000,
        },
      ],
      10_000,
    ),
    AUTOMATIONS_RUNNING_REFETCH_MS,
  );
});

test("nextAutomationsRefetchDelay picks the nearest useful refresh", () => {
  assert.equal(
    nextAutomationsRefetchDelay(
      [
        {
          state: "scheduled",
          next_run_timestamp: 25_000,
        },
        {
          state: "scheduled",
          next_run_timestamp: 10_100,
        },
        {
          state: "active",
          has_running_run: true,
          next_run_timestamp: 40_000,
        },
      ],
      10_000,
    ),
    1_300,
  );
});
