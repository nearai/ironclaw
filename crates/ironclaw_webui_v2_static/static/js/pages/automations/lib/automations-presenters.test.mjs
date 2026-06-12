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
        source: { type: "schedule", cron: "0 9 * * 1-5", timezone: "America/New_York" },
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
  assert.equal(automations[0].schedule_label, "Weekdays at 9:00 AM (America/New_York)");
  assert.equal(automations[0].schedule_timezone, "America/New_York");
  assert.equal(automations[0].last_status_label, "Done");
});

test("normalizeAutomations defaults schedule_timezone to UTC when absent", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "utc-default",
        name: "UTC default",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "scheduled",
        is_active: false,
      },
    ],
  });

  assert.equal(automations.length, 1);
  assert.equal(automations[0].schedule_timezone, "UTC");
  assert.equal(automations[0].schedule_label, "Every day at 9:00 AM (UTC)");
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

test("normalizeAutomations preserves legacy last_run_at when recent history is empty", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "legacy-run",
        name: "Legacy run",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "active",
        last_run_at: "2026-06-04T16:01:00Z",
        last_status: "ok",
        recent_runs: [],
      },
    ],
  });

  assert.equal(automations.length, 1);
  assert.match(automations[0].last_run_label, /Jun 4/);
  assert.equal(automations[0].last_status_label, "Done");
});

test("scheduleLabel presents common recurring schedules in friendly language", () => {
  assert.equal(scheduleLabel("30 14 * * *"), "Every day at 2:30 PM");
  assert.equal(scheduleLabel("0 30 14 * * *"), "Every day at 2:30 PM");
  assert.equal(scheduleLabel("00 30 14 * * * *"), "Every day at 2:30 PM");
  assert.equal(scheduleLabel("0 8 * * 1"), "Mondays at 8:00 AM");
  assert.equal(scheduleLabel("0 8 * * MON"), "Mondays at 8:00 AM");
  assert.equal(scheduleLabel("0 8 * * 7"), "Sundays at 8:00 AM");
  assert.equal(scheduleLabel("0 9 * * MON-FRI"), "Weekdays at 9:00 AM");
  assert.equal(scheduleLabel("0 17 1 * *"), "1st day of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 17 11 * *"), "11th day of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 17 12 * *"), "12th day of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 17 13 * *"), "13th day of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 0 9 1 1 * 2027"), "Jan 1, 2027 at 9:00 AM");
  assert.equal(scheduleLabel("*/5 * * * *"), "Custom schedule");
  assert.equal(scheduleLabel("* 0 9 * * *"), "Custom schedule");
  assert.equal(scheduleLabel("0 24 * * *"), "Custom schedule");
  assert.equal(scheduleLabel("0 0 32 * *"), "Custom schedule");
  assert.equal(scheduleLabel("0 0 * 13 *"), "Custom schedule");
});

test("scheduleLabel appends timezone suffix when timezone is provided", () => {
  assert.equal(scheduleLabel("0 9 * * *", "America/New_York"), "Every day at 9:00 AM (America/New_York)");
  assert.equal(scheduleLabel("0 9 * * MON-FRI", "Europe/London"), "Weekdays at 9:00 AM (Europe/London)");
  assert.equal(scheduleLabel("0 9 * * 1", "Asia/Tokyo"), "Mondays at 9:00 AM (Asia/Tokyo)");
  assert.equal(scheduleLabel("0 17 1 * *", "America/Chicago"), "1st day of each month at 5:00 PM (America/Chicago)");
  assert.equal(scheduleLabel("0 0 9 1 1 * 2027", "UTC"), "Jan 1, 2027 at 9:00 AM (UTC)");
  // Custom schedule does not append timezone suffix
  assert.equal(scheduleLabel("*/5 * * * *", "America/New_York"), "Custom schedule");
  // No timezone argument — no suffix
  assert.equal(scheduleLabel("0 9 * * *"), "Every day at 9:00 AM");
  // Null/undefined timezone — no suffix
  assert.equal(scheduleLabel("0 9 * * *", null), "Every day at 9:00 AM");
  assert.equal(scheduleLabel("0 9 * * *", undefined), "Every day at 9:00 AM");
});

test("filterAutomations, sorting, and summary use browser-visible active state", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "active",
        name: "Active",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "active",
        is_active: false,
        next_run_at: "2026-06-05T17:00:00Z",
      },
      {
        automation_id: "scheduled",
        name: "Scheduled",
        source: { type: "schedule", cron: "0 10 * * *" },
        state: "scheduled",
        is_active: false,
        next_run_at: "2026-06-05T16:00:00Z",
      },
      {
        automation_id: "paused",
        name: "Paused",
        source: { type: "schedule", cron: "0 11 * * *" },
        state: "paused",
        is_active: true,
        next_run_at: "2026-06-05T18:00:00Z",
      },
    ],
  });

  assert.deepEqual(
    automations.map((automation) => automation.automation_id),
    ["scheduled", "active", "paused"],
  );
  assert.deepEqual(
    filterAutomations(automations, "active").map((automation) => automation.automation_id),
    ["scheduled", "active"],
  );
  assert.deepEqual(
    filterAutomations(automations, "paused").map((automation) => automation.automation_id),
    ["paused"],
  );
  assert.deepEqual(automationSummary(automations), {
    scheduled: 3,
    active: 2,
    running: 0,
    failures: 0,
    nextRun: automations[0].next_run_label,
  });
});

test("automationSummary ignores unparseable next_run_at values", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "invalid-next-run",
        name: "Invalid next run",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "scheduled",
        is_active: false,
        next_run_at: "not-a-date",
      },
    ],
  });

  assert.equal(automations[0].next_run_label, "Not scheduled");
  assert.deepEqual(automationSummary(automations), {
    scheduled: 1,
    active: 1,
    running: 0,
    failures: 0,
    nextRun: null,
  });
});

test("normalizeAutomations preserves explicit unknown state even when is_active is true", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "unknown-active",
        name: "Unknown active",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "unknown",
        is_active: true,
      },
    ],
  });

  assert.equal(automations[0].state_label, "Unknown");
  assert.equal(automations[0].state_tone, "muted");
});

test("normalizeAutomations presents bounded recent run history", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "daily",
        name: "Daily summary",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "active",
        next_run_at: "2026-06-06T16:00:00Z",
        recent_runs: [
          {
            status: "error",
            fire_slot: "2026-06-04T16:00:00Z",
            submitted_at: "2026-06-04T16:00:01Z",
            completed_at: "2026-06-04T16:03:00Z",
            thread_id: "thread-error",
            run_id: "run-error",
          },
          {
            status: "running",
            fired_at: "2026-06-05T16:00:00Z",
            submitted_at: "2026-06-05T16:00:01Z",
            thread_id: "thread-running",
            run_id: "run-running",
          },
          {
            status: "ok",
            fire_slot: "2026-06-03T16:00:00Z",
            submitted_at: "2026-06-03T16:00:01Z",
            completed_at: "2026-06-03T16:02:00Z",
            thread_id: "thread-ok",
            run_id: "run-ok",
          },
        ],
      },
    ],
  });

  assert.equal(automations[0].recent_runs.length, 3);
  assert.deepEqual(
    automations[0].recent_runs.map((run) => run.run_id),
    ["run-running", "run-error", "run-ok"],
  );
  assert.equal(automations[0].has_running_run, true);
  assert.equal(automations[0].has_failed_runs, true);
  assert.equal(automations[0].latest_run.run_id, "run-running");
  assert.equal(automations[0].current_run.run_id, "run-running");
  assert.match(automations[0].last_run_label, /Jun 4/);
  assert.equal(automations[0].last_status_label, "Error");
  assert.equal(automations[0].last_status_tone, "danger");
  // Post-acceptance statuses (running/ok/error) must produce a chat_path.
  assert.equal(automations[0].recent_runs[0].chat_path, "/chat/thread-running");
  assert.equal(automations[0].recent_runs[1].chat_path, "/chat/thread-error");
  assert.equal(automations[0].recent_runs[2].chat_path, "/chat/thread-ok");
  assert.equal(automations[0].success_rate_label, "50% visible runs");
  assert.deepEqual(automationSummary(automations), {
    scheduled: 1,
    active: 1,
    running: 1,
    failures: 1,
    nextRun: automations[0].next_run_label,
  });
  assert.deepEqual(
    filterAutomations(automations, "running").map((automation) => automation.automation_id),
    ["daily"],
  );
  assert.deepEqual(
    filterAutomations(automations, "failures").map((automation) => automation.automation_id),
    ["daily"],
  );
});

test("normalizeAutomations does not emit chat_path when thread_id is absent/null", () => {
  // Pre-acceptance and pre-submit-failure runs have no canonical thread; the
  // backend serializes thread_id as null (or omits it). chat_path must be null
  // for any run that lacks a thread_id regardless of status.
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "pre-accept",
        name: "Pre-acceptance run",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "active",
        next_run_at: "2026-06-06T16:00:00Z",
        recent_runs: [
          {
            status: "error",
            fire_slot: "2026-06-05T16:00:00Z",
            submitted_at: "2026-06-05T16:00:01Z",
            // thread_id absent — pre-submit failure, no canonical thread
            run_id: "run-pre-accept-error",
          },
          {
            status: "running",
            fire_slot: "2026-06-05T17:00:00Z",
            submitted_at: "2026-06-05T17:00:01Z",
            // thread_id explicitly null — same shape as skip_serializing_if(None)
            thread_id: null,
            run_id: "run-pre-accept-running",
          },
        ],
      },
    ],
  });

  assert.equal(
    automations[0].recent_runs[0].chat_path,
    null,
    "error run without thread_id must not produce a chat_path",
  );
  assert.equal(
    automations[0].recent_runs[1].chat_path,
    null,
    "running run with null thread_id must not produce a chat_path",
  );
});

test("normalizeAutomations emits chat_path for any status when thread_id is present", () => {
  // Once thread_id is set (after fire acceptance), the panel can always link to it
  // regardless of run status. Replayed fires may also carry a canonical thread_id.
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "accepted",
        name: "Accepted run",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "active",
        next_run_at: "2026-06-06T16:00:00Z",
        recent_runs: [
          {
            status: "error",
            fire_slot: "2026-06-05T16:00:00Z",
            submitted_at: "2026-06-05T16:00:01Z",
            thread_id: "550e8400-e29b-41d4-a716-446655440000",
            run_id: "run-accepted-error",
          },
        ],
      },
    ],
  });

  assert.equal(
    automations[0].recent_runs[0].chat_path,
    "/chat/550e8400-e29b-41d4-a716-446655440000",
    "accepted run with thread_id must produce a chat_path",
  );
});
