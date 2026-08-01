/**
 * Automation-task command surface — the seam between the UI and the backend.
 *
 * Each function here is shaped exactly like the REST call it will become, so
 * wiring the real backend is a body swap (fetch) with no change to callers or
 * the returned shape. Every mutation resolves with the *server's* view of the
 * task (backend evidence), never an optimistic local guess — matching
 * `.claude/rules/tool-evidence.md`: UI success follows confirmed state.
 *
 * Intended endpoints (see ../AUTOMATION-TASKS-CONTRACT.md):
 *   GET   /api/webchat/v2/automations/tasks              → listAutomationTasks
 *   POST  /api/webchat/v2/automations/tasks/{id}/approve → approveAutomationTask
 *   PATCH /api/webchat/v2/automations/tasks/{id}         → modifyAutomationTask
 *   POST  /api/webchat/v2/automations/tasks/{id}/cancel  → cancelAutomationTask
 *   POST  /api/webchat/v2/automations/tasks/{id}/revert  → revertAutomationTask
 */
import {
  applyApprove,
  applyCancel,
  applyModify,
  applyRevert,
  isAutomated,
  rerunModified,
  type AutomationTask,
  type AutomationTaskPatch,
  MOCK_COMPLETED_TASKS,
} from "./automation-tasks";

/** Simulated network latency so pending/disabled states are exercised. */
const MOCK_LATENCY_MS = 320;

function delay<T>(value: T): Promise<T> {
  return new Promise((resolve) => {
    window.setTimeout(() => resolve(value), MOCK_LATENCY_MS);
  });
}

/**
 * Fetch the current automation tasks for the landing carousel.
 * SEAM: replace with the projection read; returns clones so callers own state.
 */
export async function listAutomationTasks(): Promise<AutomationTask[]> {
  return delay(MOCK_COMPLETED_TASKS.map((task) => ({ ...task })));
}

export async function approveAutomationTask(
  task: AutomationTask,
): Promise<AutomationTask> {
  // SEAM: POST .../approve — server runs the task and returns the automated record.
  return delay(applyApprove(task));
}

export async function modifyAutomationTask(
  task: AutomationTask,
  patch: AutomationTaskPatch,
): Promise<AutomationTask> {
  // SEAM: PATCH .../{id} — server validates the patch and returns the stored task.
  // Modifying an already-automated task re-runs it with the change and returns
  // the fresh result; modifying a suggestion just updates the proposal.
  const patched = applyModify(task, patch);
  return delay(isAutomated(task) ? rerunModified(patched) : patched);
}

export async function cancelAutomationTask(
  task: AutomationTask,
): Promise<AutomationTask> {
  // SEAM: POST .../cancel — server dismisses the suggestion.
  return delay(applyCancel(task));
}

export async function revertAutomationTask(
  task: AutomationTask,
): Promise<AutomationTask> {
  // SEAM: POST .../revert — server undoes the effect and returns the reverted record.
  return delay(applyRevert(task));
}
