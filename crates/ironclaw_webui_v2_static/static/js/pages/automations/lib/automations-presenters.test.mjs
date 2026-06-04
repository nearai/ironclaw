import assert from "node:assert/strict";
import test from "node:test";

import {
  automationSummary,
  filterAutomations,
  normalizeAutomations,
  scheduleLabel,
} from "./automations-presenters.js";

test("normalizeAutomations keeps only schedule rows and avoids raw schedule text", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "daily",
        name: "Daily summary",
        source: { type: "schedule", cron: "0 9 * * 1-5" },
        state: "active",
        is_active: true,
        next_run_at: "2026-06-05T16:00:00Z",
        last_run_at: "2026-06-04T16:01:00Z",
        last_status: "ok",
      },
      {
        automation_id: "future-webhook",
        name: "Future webhook",
        source: { type: "webhook" },
        state: "active",
        is_active: true,
      },
    ],
  });

  assert.equal(automations.length, 1);
  assert.equal(automations[0].display_name, "Daily summary");
  assert.equal(automations[0].schedule_label, "Weekdays at 9:00 AM");
  assert.equal(automations[0].last_status_label, "Done");
});

test("normalizeAutomations handles empty and malformed schedule payloads", () => {
  assert.deepEqual(normalizeAutomations(null), []);
  assert.deepEqual(normalizeAutomations({ automations: "not-an-array" }), []);

  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "partial",
        source: { type: "schedule", cron: "bad schedule value" },
        state: "unexpected",
        is_active: false,
        next_run_at: "not-a-date",
        last_run_at: null,
        created_at: null,
        last_status: "unknown",
      },
    ],
  });

  assert.equal(automations.length, 1);
  assert.equal(automations[0].display_name, "Untitled automation");
  assert.equal(automations[0].schedule_label, "Custom schedule");
  assert.equal(automations[0].state_label, "Unknown");
  assert.equal(automations[0].state_tone, "muted");
  assert.equal(automations[0].next_run_label, "Not scheduled");
  assert.equal(automations[0].last_run_label, "No runs yet");
  assert.equal(automations[0].created_label, "Unknown");
  assert.equal(automations[0].last_status_label, "No result");
  assert.equal(automations[0].last_status_tone, "muted");
});

test("scheduleLabel presents common recurring schedules in friendly language", () => {
  assert.equal(scheduleLabel("30 14 * * *"), "Every day at 2:30 PM");
  assert.equal(scheduleLabel("0 30 14 * * *"), "Every day at 2:30 PM");
  assert.equal(scheduleLabel("00 30 14 * * * *"), "Every day at 2:30 PM");
  assert.equal(scheduleLabel("0 8 * * 1"), "Mondays at 8:00 AM");
  assert.equal(scheduleLabel("0 17 1 * *"), "1st day of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 0 9 1 1 * 2027"), "Jan 1, 2027 at 9:00 AM");
  assert.equal(scheduleLabel("*/5 * * * *"), "Custom schedule");
  assert.equal(scheduleLabel("* 0 9 * * *"), "Custom schedule");
});

test("filterAutomations and automationSummary use browser-safe state fields", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "active",
        name: "Active",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "scheduled",
        is_active: true,
        next_run_at: "2026-06-05T17:00:00Z",
      },
      {
        automation_id: "paused",
        name: "Paused",
        source: { type: "schedule", cron: "0 10 * * *" },
        state: "paused",
        is_active: false,
        next_run_at: "2026-06-05T16:00:00Z",
      },
    ],
  });

  assert.deepEqual(
    filterAutomations(automations, "active").map((automation) => automation.automation_id),
    ["active"],
  );
  assert.deepEqual(
    filterAutomations(automations, "paused").map((automation) => automation.automation_id),
    ["paused"],
  );
  assert.deepEqual(automationSummary(automations), {
    scheduled: 2,
    active: 1,
    paused: 1,
    nextRun: automations[1].next_run_label,
  });
});
