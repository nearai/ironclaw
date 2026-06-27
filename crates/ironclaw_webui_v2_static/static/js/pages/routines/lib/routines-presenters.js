export function formatRoutineDate(iso) {
  if (!iso) return "Not scheduled";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "Not scheduled";
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function routineStatusTone(status, enabled = true) {
  if (!enabled || status === "disabled") return "muted";
  if (status === "active") return "signal";
  if (status === "running") return "warning";
  if (status === "failing") return "danger";
  if (status === "attention") return "danger";
  return "muted";
}

export function verificationTone(status) {
  if (status === "verified") return "success";
  if (status === "unverified") return "warning";
  return "muted";
}

export function sortRoutines(routines = []) {
  return [...routines].sort((a, b) => {
    if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
    const bTime = new Date(b.next_fire_at || b.last_run_at || 0).getTime();
    const aTime = new Date(a.next_fire_at || a.last_run_at || 0).getTime();
    return (Number.isNaN(bTime) ? 0 : bTime) - (Number.isNaN(aTime) ? 0 : aTime);
  });
}

export function summarizeRoutineAction(action) {
  if (!action || typeof action !== "object") return "No action details";
  if (action.type) return action.type;
  if (action.Lightweight) return "lightweight";
  if (action.FullJob) return "full job";
  return "configured";
}
