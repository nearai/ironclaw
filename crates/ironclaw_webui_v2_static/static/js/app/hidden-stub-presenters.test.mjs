import assert from "node:assert/strict";
import test from "node:test";

import {
  canShowCancel,
  canShowRestart,
  formatDuration,
  jobSecondaryMeta,
  stateLabel,
  statusToneForState,
  truncateJobId,
} from "../pages/jobs/lib/jobs-presenters.js";
import {
  routineStatusTone,
  sortRoutines,
  summarizeRoutineAction,
  verificationTone,
} from "../pages/routines/lib/routines-presenters.js";
import {
  missionTone,
  sortMissions,
  summarizeMissions,
} from "../pages/missions/lib/missions-presenters.js";

test("hidden jobs presenters keep empty-shell state labels and actions deterministic", () => {
  assert.equal(stateLabel(), "unknown");
  assert.equal(stateLabel("in_progress"), "in progress");
  assert.equal(statusToneForState("completed"), "success");
  assert.equal(statusToneForState("in_progress"), "signal");
  assert.equal(statusToneForState("pending"), "warning");
  assert.equal(statusToneForState("failed"), "danger");
  assert.equal(statusToneForState("not-a-state"), "muted");

  assert.equal(canShowCancel({ state: "pending" }), true);
  assert.equal(canShowCancel({ state: "completed" }), false);
  assert.equal(canShowRestart({ state: "failed", can_restart: true }), true);
  assert.equal(canShowRestart({ state: "completed", can_restart: true }), false);
  assert.equal(canShowRestart({ state: "failed", can_restart: false }), false);
  assert.equal(
    canShowRestart({ job_kind: "sandbox", state: "cancelled", can_restart: true }),
    false,
  );
  assert.equal(
    canShowRestart({ job_kind: "sandbox", state: "interrupted", can_restart: true }),
    true,
  );

  assert.equal(truncateJobId(), "unknown");
  assert.equal(truncateJobId("abcdef123456", 6), "abcdef");
  assert.equal(formatDuration(null), "Not available");
  assert.equal(formatDuration(59), "59s");
  assert.equal(formatDuration(125), "2m 5s");
  assert.equal(formatDuration(3661), "1h 1m");
  assert.equal(
    jobSecondaryMeta({
      job_kind: "workflow",
      job_mode: "acp:manual",
      started_at: "2026-06-27T10:15:00Z",
    }).startsWith("workflow job / acp manual / started "),
    true,
  );
});

test("hidden routines presenters sort enabled routines first and fail closed visibly", () => {
  assert.equal(routineStatusTone("active"), "signal");
  assert.equal(routineStatusTone("running"), "warning");
  assert.equal(routineStatusTone("failing"), "danger");
  assert.equal(routineStatusTone("active", false), "muted");
  assert.equal(verificationTone("verified"), "success");
  assert.equal(verificationTone("unverified"), "warning");
  assert.equal(verificationTone("unknown"), "muted");

  const sorted = sortRoutines([
    { id: "disabled-newer", enabled: false, next_fire_at: "2026-06-27T12:00:00Z" },
    { id: "enabled-older", enabled: true, next_fire_at: "2026-06-27T09:00:00Z" },
    { id: "enabled-newer", enabled: true, next_fire_at: "2026-06-27T11:00:00Z" },
  ]);
  assert.deepEqual(sorted.map((routine) => routine.id), [
    "enabled-newer",
    "enabled-older",
    "disabled-newer",
  ]);

  assert.equal(summarizeRoutineAction(null), "No action details");
  assert.equal(summarizeRoutineAction({ type: "notify" }), "notify");
  assert.equal(summarizeRoutineAction({ Lightweight: {} }), "lightweight");
  assert.equal(summarizeRoutineAction({ FullJob: {} }), "full job");
  assert.equal(summarizeRoutineAction({ prompt: "ship" }), "configured");
});

test("hidden missions presenters summarize and sort empty-shell mission state", () => {
  assert.equal(missionTone("Active"), "signal");
  assert.equal(missionTone("Paused"), "warning");
  assert.equal(missionTone("Completed"), "success");
  assert.equal(missionTone("Failed"), "danger");
  assert.equal(missionTone("Unknown"), "muted");

  assert.deepEqual(
    summarizeMissions([
      { status: "Active", thread_count: 2 },
      { status: "Paused", threads: ["a", "b", "c"] },
      { status: "Completed" },
      { status: "Failed", thread_count: "4" },
    ]),
    { total: 4, active: 1, paused: 1, completed: 1, failed: 1, threads: 9 },
  );

  const sorted = sortMissions([
    { id: "completed-newer", status: "Completed", updated_at: "2026-06-27T12:00:00Z" },
    { id: "active-older", status: "Active", updated_at: "2026-06-27T09:00:00Z" },
    { id: "active-newer", status: "Active", updated_at: "2026-06-27T11:00:00Z" },
    { id: "failed", status: "Failed", updated_at: "2026-06-27T10:00:00Z" },
  ]);
  assert.deepEqual(sorted.map((mission) => mission.id), [
    "active-newer",
    "active-older",
    "failed",
    "completed-newer",
  ]);
});
