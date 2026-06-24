import assert from "node:assert/strict";
import test from "node:test";

import {
  automationSummary,
  filterAutomations,
  normalizeAutomations as normalizeAutomationsRaw,
  runStatusBreakdown,
  runSummaryView,
  scheduleLabel as scheduleLabelRaw,
  summarizeRuns,
} from "./automations-presenters.js";

// Schedule labels are now localized: the presenter takes a translator + locale.
// Mirror the en.js `automations.schedule.*` templates so these tests exercise
// the real key→template→interpolation path and assert human-readable English.
const EN_SCHEDULE = {
  "automations.schedule.custom": "Custom schedule",
  "automations.schedule.everyMinute": "Every minute",
  "automations.schedule.everyMinutes": "Every {count} minutes",
  "automations.schedule.hourlyAt": "Hourly at :{minute}",
  "automations.schedule.everyDayAt": "Every day at {time}",
  "automations.schedule.weekdaysAt": "Weekdays at {time}",
  "automations.schedule.weekdayAt": "{weekday} at {time}",
  "automations.schedule.monthlyAt": "Day {day} of each month at {time}",
  "automations.schedule.dateAt": "{date} at {time}",
  "automations.schedule.onceAt": "Once on {datetime}",
  // Status / state / date-fallback labels are now localized too; mirror en.js
  // so assertions read human English.
  "automations.state.active": "Active",
  "automations.state.scheduled": "Scheduled",
  "automations.state.paused": "Paused",
  "automations.state.disabled": "Disabled",
  "automations.state.inactive": "Inactive",
  "automations.state.completed": "Completed",
  "automations.state.unknown": "Unknown",
  "automations.lastStatus.done": "Done",
  "automations.lastStatus.error": "Error",
  "automations.lastStatus.running": "Running",
  "automations.lastStatus.none": "No result",
  "automations.runStatus.ok": "OK",
  "automations.runStatus.error": "Error",
  "automations.runStatus.running": "Running",
  "automations.runStatus.unknown": "Unknown",
  "automations.status.running": "Running",
  "automations.status.needsReview": "Needs review",
  "automations.date.unknown": "Unknown",
  "automations.date.notScheduled": "Not scheduled",
  "automations.date.noRuns": "No runs yet",
  "automations.date.unscheduled": "Unscheduled",
  "automations.date.notSubmitted": "Not submitted",
  "automations.date.notCompleted": "Not completed",
  "automations.untitled": "Untitled automation",
  "automations.successRate.none": "No completed runs",
  "automations.successRate.visible": "{percent}% visible runs",
  "automations.filter.completed": "Completed",
  // Run-summary labels (mirror en.js) so runSummaryView assertions read English.
  "automations.runs.total": "Recent runs: {count}",
  "automations.runs.ok": "OK: {count}",
  "automations.runs.error": "Failed: {count}",
  "automations.runs.running": "Running: {count}",
  "automations.runs.unknown": "Unknown: {count}",
};
const t = (key, params = {}) =>
  (EN_SCHEDULE[key] || key).replace(/\{(\w+)\}/g, (m, k) =>
    params[k] !== undefined ? params[k] : m,
  );
// Intl may separate the clock time from AM/PM with a narrow no-break space
// (U+202F) depending on the ICU build; normalize so assertions are stable.
const norm = (value) =>
  typeof value === "string" ? value.replace(/[\u202f\u00a0]/g, " ") : value;
const scheduleLabel = (cron, timezone, locale = "en") =>
  norm(scheduleLabelRaw(cron, timezone, t, locale));
const normalizeAutomations = (response, locale = "en") =>
  normalizeAutomationsRaw(response, t, locale).map((automation) => ({
    ...automation,
    schedule_label: norm(automation.schedule_label),
  }));

test("normalizeAutomations keeps schedule and once rows, drops unknown", () => {
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
        automation_id: "fire-once",
        name: "One-shot task",
        source: { type: "once", at: "2026-06-25T14:00:00Z", timezone: "America/New_York" },
        state: "scheduled",
        is_active: false,
        next_run_at: "2026-06-25T14:00:00Z",
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

  // webhook row is dropped; schedule + once rows are kept
  assert.equal(automations.length, 2);

  const scheduleRow = automations.find((a) => a.automation_id === "daily");
  assert.ok(scheduleRow, "schedule row must be present");
  assert.equal(scheduleRow.schedule_label, "Weekdays at 9:00 AM (America/New_York)");
  assert.equal(scheduleRow.schedule_timezone, "America/New_York");
  assert.equal(scheduleRow.last_status_label, "Done");

  const onceRow = automations.find((a) => a.automation_id === "fire-once");
  assert.ok(onceRow, "once row must be present");
  // Must contain the localized date AND the tz parenthetical
  assert.match(onceRow.schedule_label, /Once on /);
  assert.match(onceRow.schedule_label, /\(America\/New_York\)/);
  assert.notEqual(onceRow.schedule_label, "Custom schedule");

  // unknown (webhook) must be dropped
  assert.ok(!automations.find((a) => a.automation_id === "future-webhook"), "webhook row must be dropped");
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
  assert.equal(scheduleLabel("0 8 * * 1"), "Monday at 8:00 AM");
  assert.equal(scheduleLabel("0 8 * * MON"), "Monday at 8:00 AM");
  assert.equal(scheduleLabel("0 8 * * 7"), "Sunday at 8:00 AM");
  assert.equal(scheduleLabel("0 9 * * MON-FRI"), "Weekdays at 9:00 AM");
  assert.equal(scheduleLabel("0 17 1 * *"), "Day 1 of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 17 11 * *"), "Day 11 of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 17 12 * *"), "Day 12 of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 17 13 * *"), "Day 13 of each month at 5:00 PM");
  assert.equal(scheduleLabel("0 0 9 1 1 * 2027"), "Jan 1, 2027 at 9:00 AM");
  // Feb 29 with no year must not roll over to Mar 1 (placeholder year must be
  // a leap year).
  assert.equal(scheduleLabel("0 0 29 2 *"), "Feb 29 at 12:00 AM");
  assert.equal(scheduleLabel("* 0 9 * * *"), "Custom schedule");
  assert.equal(scheduleLabel("0 24 * * *"), "Custom schedule");
  assert.equal(scheduleLabel("0 0 32 * *"), "Custom schedule");
  assert.equal(scheduleLabel("0 0 * 13 *"), "Custom schedule");
});

test("scheduleLabel labels sub-hourly and hourly cadences", () => {
  assert.equal(scheduleLabel("* * * * *"), "Every minute");
  assert.equal(scheduleLabel("*/1 * * * *"), "Every minute");
  assert.equal(scheduleLabel("*/5 * * * *"), "Every 5 minutes");
  assert.equal(scheduleLabel("*/15 * * * *"), "Every 15 minutes");
  assert.equal(scheduleLabel("0 * * * *"), "Hourly at :00");
  assert.equal(scheduleLabel("30 * * * *"), "Hourly at :30");
  // Minute/hour cadences are timezone-independent — no suffix appended.
  assert.equal(scheduleLabel("*/15 * * * *", "America/New_York"), "Every 15 minutes");
});

test("scheduleLabel appends timezone suffix when timezone is provided", () => {
  assert.equal(scheduleLabel("0 9 * * *", "America/New_York"), "Every day at 9:00 AM (America/New_York)");
  assert.equal(scheduleLabel("0 9 * * MON-FRI", "Europe/London"), "Weekdays at 9:00 AM (Europe/London)");
  assert.equal(scheduleLabel("0 9 * * 1", "Asia/Tokyo"), "Monday at 9:00 AM (Asia/Tokyo)");
  assert.equal(scheduleLabel("0 17 1 * *", "America/Chicago"), "Day 1 of each month at 5:00 PM (America/Chicago)");
  assert.equal(scheduleLabel("0 0 9 1 1 * 2027", "UTC"), "Jan 1, 2027 at 9:00 AM (UTC)");
  // Custom schedule does not append timezone suffix
  assert.equal(scheduleLabel("* 0 9 * * *", "America/New_York"), "Custom schedule");
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
  assert.equal(automations[0].primary_status_label, "Running");
  assert.equal(automations[0].primary_status_tone, "info");
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

test("normalizeAutomations keeps paused primary status even with a running recent run", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "paused-with-run",
        name: "Paused with run",
        source: { type: "schedule", cron: "* * * * *" },
        state: "paused",
        next_run_at: "2026-06-06T16:00:00Z",
        recent_runs: [
          {
            status: "running",
            fired_at: "2026-06-05T16:00:00Z",
            submitted_at: "2026-06-05T16:00:01Z",
            thread_id: "thread-running",
            run_id: "run-running",
          },
        ],
      },
    ],
  });

  assert.equal(automations[0].state_label, "Paused");
  assert.equal(automations[0].state_tone, "warning");
  assert.equal(automations[0].has_running_run, true);
  assert.equal(automations[0].current_run.run_id, "run-running");
  assert.equal(automations[0].primary_status_label, "Paused");
  assert.equal(automations[0].primary_status_tone, "warning");
  assert.deepEqual(
    filterAutomations(automations, "paused").map((automation) => automation.automation_id),
    ["paused-with-run"],
  );
  assert.deepEqual(
    filterAutomations(automations, "running").map((automation) => automation.automation_id),
    ["paused-with-run"],
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

test("summarizeRuns counts runs by normalized status", () => {
  const counts = summarizeRuns([
    { status: "ok" },
    { status: "ok" },
    { status: "error" },
    { status: "running" },
    { status: "weird-unknown-status" },
    {},
  ]);
  assert.deepEqual(counts, {
    total: 6,
    ok: 2,
    error: 1,
    running: 1,
    // both the unrecognized status and the missing status fold into "unknown"
    unknown: 2,
  });
});

test("summarizeRuns tolerates non-array input", () => {
  assert.deepEqual(summarizeRuns(undefined), {
    total: 0,
    ok: 0,
    error: 0,
    running: 0,
    unknown: 0,
  });
});

// Regression for #4988: the recent-run summary chips render straight from
// runStatusBreakdown. Every counted status — including the `unknown` bucket
// produced by unrecognized/missing statuses — must appear, so a future edit
// can't silently drop a bucket from what the UI shows.
test("runStatusBreakdown surfaces every non-empty bucket incl. unknown", () => {
  const parts = runStatusBreakdown([
    { status: "ok" },
    { status: "ok" },
    { status: "error" },
    { status: "running" },
    { status: "mystery-status" }, // unrecognized -> unknown
    {}, // missing -> unknown
  ]);
  const byKey = Object.fromEntries(parts.map((part) => [part.key, part.count]));
  assert.deepEqual(byKey, { ok: 2, error: 1, running: 1, unknown: 2 });
  assert.ok(
    parts.some((part) => part.key === "unknown"),
    "unknown bucket must be present when unknown-status runs exist",
  );
  // Chips must sum to the same total the summary header shows.
  const total = parts.reduce((sum, part) => sum + part.count, 0);
  assert.equal(total, summarizeRuns([
    { status: "ok" },
    { status: "ok" },
    { status: "error" },
    { status: "running" },
    { status: "mystery-status" },
    {},
  ]).total);
});

test("runStatusBreakdown omits zero-count buckets", () => {
  const parts = runStatusBreakdown([{ status: "ok" }]);
  assert.deepEqual(parts.map((part) => part.key), ["ok"]);
});

// Drives the exact data RunHistorySummary renders (the component maps this 1:1,
// with no logic of its own). With an unrecognized + a missing status present,
// the rendered chips must include the localized unknown chip and must sum to
// the total shown in the header. Guards #4988 at the render path, not just the
// counting helper.
test("runSummaryView renders the unknown chip and chips sum to total", () => {
  const runs = [
    { status: "ok" },
    { status: "ok" },
    { status: "error" },
    { status: "running" },
    { status: "mystery-status" }, // unrecognized -> unknown
    {}, // missing -> unknown
  ];
  const view = runSummaryView(runs, t);

  assert.equal(view.total, 6);
  assert.equal(view.totalText, "Recent runs: 6");

  const unknownChip = view.chips.find((chip) => chip.key === "unknown");
  assert.ok(unknownChip, "rendered chips must include the unknown bucket");
  assert.equal(unknownChip.text, "Unknown: 2");

  const renderedTexts = view.chips.map((chip) => chip.text);
  assert.deepEqual(renderedTexts, [
    "OK: 2",
    "Failed: 1",
    "Running: 1",
    "Unknown: 2",
  ]);

  const chipSum = view.chips.reduce((sum, chip) => sum + chip.count, 0);
  assert.equal(chipSum, view.total, "displayed chips must sum to the total");
});

test("runSummaryView reports zero total for an empty history", () => {
  const view = runSummaryView([], t);
  assert.equal(view.total, 0);
  assert.deepEqual(view.chips, []);
});

test("once automation with valid at produces label with date and tz, not Custom schedule", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "once-valid",
        name: "Send report",
        source: { type: "once", at: "2026-06-25T14:00:00Z", timezone: "Europe/Berlin" },
        state: "scheduled",
        is_active: false,
        next_run_at: "2026-06-25T14:00:00Z",
      },
    ],
  });

  assert.equal(automations.length, 1);
  const label = automations[0].schedule_label;
  assert.match(label, /Once on /);
  assert.match(label, /\(Europe\/Berlin\)/);
  assert.notEqual(label, "Custom schedule");
});

test("once automation with missing at falls back to Custom schedule", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "once-no-at",
        name: "Missing at",
        source: { type: "once", timezone: "UTC" },
        state: "scheduled",
        is_active: false,
      },
    ],
  });

  assert.equal(automations.length, 1);
  assert.equal(automations[0].schedule_label, "Custom schedule");
});

test("AUTOMATION_FILTERS contains a completed entry whose predicate matches only completed rows", () => {
  // Drive via filterAutomations (which delegates to the predicate in AUTOMATION_FILTERS)
  // so the test exercises the full filter dispatch without re-importing the module.
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "scheduled-one",
        name: "Scheduled",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "scheduled",
        is_active: false,
        next_run_at: "2026-06-25T09:00:00Z",
      },
      {
        automation_id: "completed-one",
        name: "Completed one-shot",
        source: { type: "once", at: "2026-06-10T12:00:00Z", timezone: "UTC" },
        state: "completed",
        is_active: false,
      },
    ],
  });

  const completedOnly = filterAutomations(automations, "completed");
  assert.equal(completedOnly.length, 1, "predicate must keep only completed rows");
  assert.equal(completedOnly[0].automation_id, "completed-one");

  const scheduledOnly = filterAutomations(automations, "active");
  assert.ok(
    !scheduledOnly.some((a) => a.automation_id === "completed-one"),
    "completed row must not appear in the active filter",
  );
});

test("automationSummary excludes completed rows from scheduled count", () => {
  const automations = normalizeAutomations({
    automations: [
      {
        automation_id: "scheduled-active",
        name: "Active schedule",
        source: { type: "schedule", cron: "0 9 * * *" },
        state: "scheduled",
        is_active: false,
        next_run_at: "2026-06-25T09:00:00Z",
      },
      {
        automation_id: "soft-completed",
        name: "Fired one-shot",
        source: { type: "once", at: "2026-06-10T12:00:00Z", timezone: "UTC" },
        state: "completed",
        is_active: false,
      },
    ],
  });

  const summary = automationSummary(automations);
  assert.equal(summary.scheduled, 1, "completed row must not count toward scheduled total");
});

test("once label reflects source timezone wall-clock, not UTC", () => {
  // at="2026-06-24T00:30:00Z" is Jun 23 (previous evening) in LA (UTC-7 in
  // summer), so the LA wall-clock label must differ from the UTC-rendered label.
  const at = "2026-06-24T00:30:00Z";
  const timezone = "America/Los_Angeles";

  const automationsLa = normalizeAutomations({
    automations: [
      {
        automation_id: "once-la",
        name: "LA once",
        source: { type: "once", at, timezone },
        state: "scheduled",
        is_active: false,
        next_run_at: at,
      },
    ],
  });

  const automationsUtc = normalizeAutomations({
    automations: [
      {
        automation_id: "once-utc",
        name: "UTC once",
        source: { type: "once", at, timezone: "UTC" },
        state: "scheduled",
        is_active: false,
        next_run_at: at,
      },
    ],
  });

  const laLabel = automationsLa[0].schedule_label;
  const utcLabel = automationsUtc[0].schedule_label;

  // Both are valid once labels (not fallbacks)
  assert.match(laLabel, /Once on /);
  assert.match(utcLabel, /Once on /);

  // The rendered date/time portion must differ because the tz wall-clocks differ
  // (Jun 23 evening in LA vs Jun 24 morning in UTC). This proves timeZone is
  // applied, not just the runner's local tz.
  assert.notEqual(laLabel, utcLabel, "LA and UTC labels must differ for a cross-midnight instant");

  // LA label must carry the LA tz parenthetical
  assert.match(laLabel, /\(America\/Los_Angeles\)/);
});
