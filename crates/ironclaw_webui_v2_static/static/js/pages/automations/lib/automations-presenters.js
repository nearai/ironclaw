const WEEKDAYS = [
  "Sundays",
  "Mondays",
  "Tuesdays",
  "Wednesdays",
  "Thursdays",
  "Fridays",
  "Saturdays",
];
const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

const STATE_PRESENTATION = {
  active: { label: "Active", tone: "signal" },
  scheduled: { label: "Scheduled", tone: "signal" },
  paused: { label: "Paused", tone: "warning" },
  disabled: { label: "Disabled", tone: "warning" },
  inactive: { label: "Inactive", tone: "warning" },
  completed: { label: "Completed", tone: "success" },
  unknown: { label: "Unknown", tone: "muted" },
};

const LAST_STATUS_PRESENTATION = {
  ok: { label: "Done", tone: "success" },
  error: { label: "Error", tone: "danger" },
  running: { label: "Running", tone: "info" },
};

const RUN_STATUS_PRESENTATION = {
  ok: { label: "OK", tone: "success" },
  error: { label: "Error", tone: "danger" },
  running: { label: "Running", tone: "info" },
  unknown: { label: "Unknown", tone: "muted" },
};

export const AUTOMATION_FILTERS = [
  { value: "all", labelKey: "automations.filter.all", predicate: null },
  { value: "active", labelKey: "automations.filter.active", predicate: isBrowserActive },
  {
    value: "running",
    labelKey: "automations.filter.running",
    predicate: (automation) => automation.has_running_run,
  },
  {
    value: "failures",
    labelKey: "automations.filter.failures",
    predicate: (automation) => automation.has_failed_runs,
  },
  { value: "paused", labelKey: "automations.filter.paused", predicate: isBrowserPaused },
];

export function normalizeAutomations(response) {
  const automations = Array.isArray(response?.automations)
    ? response.automations
    : [];
  return automations
    .filter((automation) => automation?.source?.type === "schedule")
    .map((automation) => normalizeAutomation(automation))
    .sort(compareAutomations);
}

export function filterAutomations(automations, filter) {
  const strategy = AUTOMATION_FILTERS.find((item) => item.value === filter)?.predicate;
  return strategy ? automations.filter(strategy) : automations;
}

export function automationSummary(automations) {
  const active = automations.filter((automation) => isBrowserActive(automation)).length;
  const running = automations.reduce(
    (count, automation) =>
      count + automation.recent_runs.filter((run) => run.status === "running").length,
    0,
  );
  const failures = automations.reduce(
    (count, automation) =>
      count + automation.recent_runs.filter((run) => run.status === "error").length,
    0,
  );
  const next = automations
    .filter((automation) => nextRunTimestamp(automation) !== null)
    .sort(
      (a, b) =>
        (a.next_run_timestamp ?? Number.MAX_SAFE_INTEGER) -
        (b.next_run_timestamp ?? Number.MAX_SAFE_INTEGER),
    )[0];
  return {
    scheduled: automations.length,
    active,
    running,
    failures,
    nextRun: next?.next_run_label || null,
  };
}

export function scheduleLabel(cron) {
  if (!cron || typeof cron !== "string") return "Custom schedule";
  const parts = cronFields(cron);
  if (!parts) return "Custom schedule";

  const { minute, hour, dayOfMonth, month, dayOfWeek, year } = parts;
  const time = formatCronTime(hour, minute);
  if (!time) return "Custom schedule";

  if (year === "*" && dayOfMonth === "*" && month === "*" && dayOfWeek === "*") {
    return `Every day at ${time}`;
  }
  const normalizedDayOfWeek = normalizeDayOfWeek(dayOfWeek);

  if (year === "*" && dayOfMonth === "*" && month === "*" && normalizedDayOfWeek === "1-5") {
    return `Weekdays at ${time}`;
  }
  if (
    year === "*" &&
    dayOfMonth === "*" &&
    month === "*" &&
    isSingleNumber(normalizedDayOfWeek, 0, 7)
  ) {
    return `${WEEKDAYS[Number(normalizedDayOfWeek) % 7]} at ${time}`;
  }
  if (
    year === "*" &&
    isSingleNumber(dayOfMonth, 1, 31) &&
    month === "*" &&
    dayOfWeek === "*"
  ) {
    return `${ordinal(Number(dayOfMonth))} day of each month at ${time}`;
  }
  if (
    isSingleNumber(dayOfMonth, 1, 31) &&
    isSingleNumber(month, 1, 12) &&
    dayOfWeek === "*" &&
    (year === "*" || isSingleNumber(year, 1970, 9999))
  ) {
    const date = `${MONTHS[Number(month) - 1]} ${Number(dayOfMonth)}`;
    return year === "*" ? `${date} at ${time}` : `${date}, ${year} at ${time}`;
  }

  return "Custom schedule";
}

export function formatAutomationDate(value, fallback = "Unknown") {
  if (!value) return fallback;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return fallback;
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function stateLabel(state) {
  return STATE_PRESENTATION[state]?.label || "Unknown";
}

export function stateTone(state) {
  return STATE_PRESENTATION[state]?.tone || "muted";
}

export function lastStatusLabel(status) {
  return LAST_STATUS_PRESENTATION[status]?.label || "No result";
}

export function lastStatusTone(status) {
  return LAST_STATUS_PRESENTATION[status]?.tone || "muted";
}

export function runStatusLabel(status) {
  return RUN_STATUS_PRESENTATION[normalizeRunStatus(status)]?.label || "Unknown";
}

export function runStatusTone(status) {
  return RUN_STATUS_PRESENTATION[normalizeRunStatus(status)]?.tone || "muted";
}

function normalizeAutomation(automation) {
  const recentRuns = normalizeRuns(automation.recent_runs);
  const latestRun = recentRuns[0] || null;
  const currentRun = recentRuns.find((run) => run.status === "running") || null;
  const lastCompletedRun =
    recentRuns.find((run) => run.status === "ok" || run.status === "error") ||
    null;
  const lastStatus = lastCompletedRun?.status || automation.last_status;
  const lastRunAt = lastCompletedRun?.completed_at || automation.last_run_at || null;

  return {
    ...automation,
    display_name: automation.name || "Untitled automation",
    schedule_label: scheduleLabel(automation.source?.cron),
    state_label: stateLabel(automation.state),
    state_tone: stateTone(automation.state),
    next_run_timestamp: parseTimestamp(automation.next_run_at),
    next_run_label: formatAutomationDate(automation.next_run_at, "Not scheduled"),
    last_run_label: formatAutomationDate(lastRunAt, "No runs yet"),
    last_status_label: lastStatusLabel(lastStatus),
    last_status_tone: lastStatusTone(lastStatus),
    created_label: formatAutomationDate(automation.created_at, "Unknown"),
    recent_runs: recentRuns,
    latest_run: latestRun,
    current_run: currentRun,
    has_running_run: recentRuns.some((run) => run.status === "running"),
    has_failed_runs: recentRuns.some((run) => run.status === "error"),
    success_rate_label: successRateLabel(recentRuns),
  };
}

function normalizeRuns(runs) {
  if (!Array.isArray(runs)) return [];
  return runs
    .map((run) => {
      const status = normalizeRunStatus(run?.status);
      const timestampSource =
        run?.fired_at || run?.fire_slot || run?.submitted_at || run?.completed_at || null;
      const timestamp = parseTimestamp(timestampSource);
      return {
        ...run,
        status,
        status_label: runStatusLabel(status),
        status_tone: runStatusTone(status),
        timestamp,
        timestamp_source: timestampSource,
        fired_label: formatAutomationDate(timestampSource, "Unscheduled"),
        submitted_label: formatAutomationDate(run?.submitted_at, "Not submitted"),
        completed_label: formatAutomationDate(run?.completed_at, "Not completed"),
        chat_path: run?.thread_id ? `/chat/${encodeURIComponent(run.thread_id)}` : null,
      };
    })
    .sort((a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0));
}

function normalizeRunStatus(status) {
  if (status === "ok" || status === "error" || status === "running") return status;
  return "unknown";
}

function successRateLabel(runs) {
  const terminalRuns = runs.filter((run) => run.status === "ok" || run.status === "error");
  if (!terminalRuns.length) return "No completed runs";
  const ok = terminalRuns.filter((run) => run.status === "ok").length;
  return `${Math.round((ok / terminalRuns.length) * 100)}% visible runs`;
}

function compareAutomations(a, b) {
  const aActive = isBrowserActive(a);
  const bActive = isBrowserActive(b);
  if (aActive !== bActive) return aActive ? -1 : 1;
  return (nextRunTimestamp(a) ?? Number.MAX_SAFE_INTEGER) -
    (nextRunTimestamp(b) ?? Number.MAX_SAFE_INTEGER);
}

function parseTimestamp(value) {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date.getTime();
}

function isBrowserActive(automation) {
  return automation?.state === "active" || automation?.state === "scheduled";
}

function isBrowserPaused(automation) {
  return ["paused", "disabled", "inactive"].includes(automation?.state);
}

function nextRunTimestamp(automation) {
  return automation?.next_run_timestamp ?? parseTimestamp(automation?.next_run_at);
}

function formatCronTime(hour, minute) {
  if (!isSingleNumber(hour, 0, 23) || !isSingleNumber(minute, 0, 59)) return null;
  const hourNum = Number(hour);
  const minuteNum = Number(minute);
  const period = hourNum >= 12 ? "PM" : "AM";
  const displayHour = hourNum % 12 || 12;
  return `${displayHour}:${String(minuteNum).padStart(2, "0")} ${period}`;
}

function cronFields(cron) {
  const fields = cron.trim().split(/\s+/);
  if (fields.length === 5) {
    const [minute, hour, dayOfMonth, month, dayOfWeek] = fields;
    return { minute, hour, dayOfMonth, month, dayOfWeek, year: "*" };
  }
  if (fields.length === 6 && isZeroSeconds(fields[0])) {
    const [, minute, hour, dayOfMonth, month, dayOfWeek] = fields;
    return { minute, hour, dayOfMonth, month, dayOfWeek, year: "*" };
  }
  if (fields.length === 7 && isZeroSeconds(fields[0])) {
    const [, minute, hour, dayOfMonth, month, dayOfWeek, year] = fields;
    return { minute, hour, dayOfMonth, month, dayOfWeek, year };
  }
  return null;
}

function isZeroSeconds(value) {
  return /^0+$/.test(value);
}

function isSingleNumber(value, min, max) {
  if (!/^\d+$/.test(value)) return false;
  const num = Number(value);
  return num >= min && num <= max;
}

function normalizeDayOfWeek(value) {
  const upper = String(value || "").toUpperCase();
  const aliases = {
    SUN: "0",
    MON: "1",
    TUE: "2",
    WED: "3",
    THU: "4",
    FRI: "5",
    SAT: "6",
    "MON-FRI": "1-5",
  };
  return aliases[upper] || value;
}

function ordinal(value) {
  const mod100 = value % 100;
  if (mod100 >= 11 && mod100 <= 13) return `${value}th`;
  if (value % 10 === 1) return `${value}st`;
  if (value % 10 === 2) return `${value}nd`;
  if (value % 10 === 3) return `${value}rd`;
  return `${value}th`;
}
