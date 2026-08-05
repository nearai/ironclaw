export const JOB_DETAIL_TABS = [
  { id: "overview", label: "Overview" },
  { id: "activity", label: "Activity" },
  { id: "files", label: "Files" },
];

const ACTIVE_STATES = new Set(["pending", "in_progress"]);
const FAILURE_STATES = new Set(["failed", "interrupted", "stuck", "cancelled"]);

export function stateLabel(state) {
  if (!state) return "unknown";
  return String(state).replace(/_/g, " ");
}

export function statusToneForState(state) {
  if (!state) return "muted";
  if (state === "completed" || state === "accepted" || state === "submitted") return "success";
  if (state === "in_progress") return "signal";
  if (state === "pending") return "warning";
  if (FAILURE_STATES.has(state)) return "danger";
  return "muted";
}

export function isActiveJobState(state) {
  return ACTIVE_STATES.has(state);
}

export function canShowCancel(job) {
  return isActiveJobState(job?.state);
}

export function canShowRestart(job) {
  if (!job?.can_restart) return false;
  if (job.job_kind === "sandbox") {
    return job.state === "failed" || job.state === "interrupted";
  }
  return FAILURE_STATES.has(job.state);
}

export function truncateJobId(id, length = 8) {
  return id ? String(id).slice(0, length) : "unknown";
}

export function formatJobDate(iso, options = {}) {
  if (!iso) return "Not available";
  return new Date(iso).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    ...options,
  });
}

export function formatDuration(seconds) {
  if (seconds == null) return "Not available";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes < 60) {
    return `${minutes}m ${remainingSeconds}s`;
  }
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export function jobSecondaryMeta(job) {
  const parts = [
    job?.job_kind ? `${job.job_kind} job` : null,
    job?.job_mode ? job.job_mode.replace(/^acp:/, "acp ") : null,
    job?.started_at ? `started ${formatJobDate(job.started_at)}` : null,
  ];
  return parts.filter(Boolean).join(" / ");
}
