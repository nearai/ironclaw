export const AUTOMATIONS_BASE_REFETCH_MS = 30_000;
export const AUTOMATIONS_RUNNING_REFETCH_MS = 5_000;
export const AUTOMATIONS_OVERDUE_REFETCH_MS = 2_000;
export const AUTOMATIONS_DUE_GRACE_MS = 1_200;
export const AUTOMATIONS_MIN_REFETCH_MS = 1_000;

function isSchedulableState(state) {
  return state === "active" || state === "scheduled";
}

function finiteTimestamp(value) {
  return Number.isFinite(value) ? value : null;
}

export function nextAutomationsRefetchDelay(automations, nowMs = Date.now()) {
  const list = Array.isArray(automations) ? automations : [];
  let delay = null;

  for (const automation of list) {
    if (!automation) continue;

    if (automation.has_running_run) {
      delay =
        delay == null
          ? AUTOMATIONS_RUNNING_REFETCH_MS
          : Math.min(delay, AUTOMATIONS_RUNNING_REFETCH_MS);
    }

    if (!isSchedulableState(automation.state)) continue;

    const nextRunAt = finiteTimestamp(automation.next_run_timestamp);
    if (nextRunAt == null) continue;

    const untilNextRun = nextRunAt - nowMs;
    const candidate =
      untilNextRun <= 0
        ? AUTOMATIONS_OVERDUE_REFETCH_MS
        : untilNextRun < AUTOMATIONS_BASE_REFETCH_MS
          ? Math.max(
              AUTOMATIONS_MIN_REFETCH_MS,
              untilNextRun + AUTOMATIONS_DUE_GRACE_MS,
            )
          : null;

    if (candidate != null) {
      delay = delay == null ? candidate : Math.min(delay, candidate);
    }
  }

  return delay;
}
