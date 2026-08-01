import assert from "node:assert/strict";
import { test } from "vitest";

import {
  applyApprove,
  applyCancel,
  applyModify,
  applyRevert,
  isAutomated,
  isSuggested,
  MOCK_AUTOMATED_RESCHEDULE,
  MOCK_COMPLETED_TASKS,
  MOCK_PLAN,
  MOCK_PLAN_TASKS,
  MOCK_SUGGESTED_RESCHEDULE,
  rerunModified,
  type AutomationTask,
} from "./automation-tasks";

function suggestedTask(): AutomationTask {
  return { ...MOCK_SUGGESTED_RESCHEDULE, reschedule: { ...MOCK_SUGGESTED_RESCHEDULE.reschedule } };
}

test("applyApprove moves a suggested task to automated and stamps completion", () => {
  const approved = applyApprove(suggestedTask());
  assert.equal(approved.state, "automated");
  assert.ok(approved.completedAt, "approved task records a completion time");
});

test("applyApprove is a no-op on a task that is not suggested", () => {
  const automated = { ...MOCK_AUTOMATED_RESCHEDULE };
  assert.equal(applyApprove(automated).state, "automated");
});

test("applyRevert only reverts an automated task", () => {
  assert.equal(applyRevert({ ...MOCK_AUTOMATED_RESCHEDULE }).state, "reverted");
  // A suggested task cannot be reverted — it was never carried out.
  assert.equal(applyRevert(suggestedTask()).state, "suggested");
});

test("applyCancel dismisses a suggestion", () => {
  assert.equal(applyCancel(suggestedTask()).state, "cancelled");
});

test("applyModify swaps the reschedule destination slot without dropping fields", () => {
  const task = suggestedTask();
  const newSlot = task.reschedule.alternativeSlots[1];
  const modified = applyModify(task, { reschedule: { to: newSlot }, note: "keep it short" });
  assert.deepEqual(modified.reschedule.to, newSlot);
  assert.equal(modified.reschedule.note, "keep it short");
  // Untouched fields survive the patch.
  assert.equal(modified.reschedule.meetingTitle, task.reschedule.meetingTitle);
  assert.equal(modified.reschedule.conflictWith, task.reschedule.conflictWith);
  // The patch does not mutate the original task.
  assert.notEqual(modified, task);
  assert.deepEqual(task.reschedule.to, MOCK_SUGGESTED_RESCHEDULE.reschedule.to);
});

test("applyModify replaces the drafted email set", () => {
  const base = MOCK_COMPLETED_TASKS.find((t) => t.kind === "email_triage");
  assert.ok(base, "a mock email-triage task exists");
  const trimmed = (base.emails ?? []).slice(0, 1).map((e) => ({ ...e, include: false }));
  const modified = applyModify({ ...base }, { emails: trimmed });
  assert.equal(modified.emails?.length, 1);
  assert.equal(modified.emails?.[0].include, false);
});

test("rerunModified re-stamps an automated task and recomputes email metrics", () => {
  const base = MOCK_COMPLETED_TASKS.find((t) => t.kind === "email_triage");
  assert.ok(base, "a mock email-triage task exists");
  // Drop one reply from the send set, then re-run.
  const emails = (base.emails ?? []).map((email, i) =>
    i === 0 ? { ...email, include: false } : email,
  );
  const rerun = rerunModified(applyModify({ ...base }, { emails }));
  assert.equal(rerun.completedAt, "Just now", "re-run refreshes the completion stamp");
  const replied = rerun.metrics?.find((m) => m.labelKey === "automation.metric.replied");
  const includedCount = emails.filter((e) => e.include).length;
  assert.equal(replied?.value, String(includedCount), "replied metric follows the send set");
});

test("rerunModified is a no-op on a task that is not automated", () => {
  const suggested = { ...MOCK_SUGGESTED_RESCHEDULE };
  assert.deepEqual(rerunModified(suggested), suggested);
});

test("plan fixtures are all suggested items ready for batch approval", () => {
  assert.ok(MOCK_PLAN_TASKS.length >= 2, "a plan batches multiple tasks");
  for (const task of MOCK_PLAN_TASKS) {
    assert.equal(task.state, "suggested", `${task.id} awaits approval`);
  }
  assert.ok(MOCK_PLAN.title && MOCK_PLAN.summary, "plan has a title and summary");
});

test("isSuggested / isAutomated classify the lifecycle states", () => {
  assert.equal(isSuggested({ ...MOCK_SUGGESTED_RESCHEDULE }), true);
  assert.equal(isSuggested({ ...MOCK_SUGGESTED_RESCHEDULE, state: "in_progress" }), true);
  assert.equal(isSuggested({ ...MOCK_AUTOMATED_RESCHEDULE }), false);
  assert.equal(isAutomated({ ...MOCK_AUTOMATED_RESCHEDULE }), true);
  assert.equal(isAutomated({ ...MOCK_SUGGESTED_RESCHEDULE }), false);
});

test("mock fixtures uphold the invariants the UI relies on", () => {
  assert.ok(MOCK_COMPLETED_TASKS.length > 0, "carousel has completed tasks");
  for (const task of MOCK_COMPLETED_TASKS) {
    assert.equal(task.state, "automated", `${task.id} is an automated completion`);
    assert.ok(task.launchUrl, `${task.id} is launchable into its app`);
  }
  assert.equal(MOCK_SUGGESTED_RESCHEDULE.state, "suggested");
  assert.ok(MOCK_SUGGESTED_RESCHEDULE.reschedule?.alternativeSlots.length);
  assert.equal(MOCK_AUTOMATED_RESCHEDULE.state, "automated");
});
