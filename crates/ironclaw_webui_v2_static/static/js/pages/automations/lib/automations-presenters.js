// Source types the presenter understands. Rows with other source types are
// silently excluded from the list (the original intent of the "schedule"-only
// guard) — adding a new backend source type requires a one-line addition here.
// Add new source types here as the backend gains them
const SUPPORTED_SOURCE_TYPES = ["schedule", "once"];

// Display tone + i18n label key for each status. The label text itself is
// resolved through `t` at render time so non-English locales don't see English
// status pills (RUNNING / ERROR / etc.).
const STATE_PRESENTATION = {
  active: { labelKey: "automations.state.active", tone: "signal" },
  scheduled: { labelKey: "automations.state.scheduled", tone: "signal" },
  paused: { labelKey: "automations.state.paused", tone: "warning" },
  disabled: { labelKey: "automations.state.disabled", tone: "warning" },
  inactive: { labelKey: "automations.state.inactive", tone: "warning" },
  completed: { labelKey: "automations.state.completed", tone: "success" },
  unknown: { labelKey: "automations.state.unknown", tone: "muted" },
};

const LAST_STATUS_PRESENTATION = {
  ok: { labelKey: "automations.lastStatus.done", tone: "success" },
  error: { labelKey: "automations.lastStatus.error", tone: "danger" },
  running: { labelKey: "automations.lastStatus.running", tone: "info" },
};

const RUN_STATUS_PRESENTATION = {
  ok: { labelKey: "automations.runStatus.ok", tone: "success" },
  error: { labelKey: "automations.runStatus.error", tone: "danger" },
  running: { labelKey: "automations.runStatus.running", tone: "info" },
  unknown: { labelKey: "automations.runStatus.unknown", tone: "muted" },
};

// Fallback translator: if a caller forgets to pass `t`, return the raw key
// rather than crash. Production paths always thread the real translator.
function tr(t) {
  return typeof t === "function" ? t : (key) => key;
}

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
  { value: "completed", labelKey: "automations.filter.completed", predicate: isBrowserCompleted },
];

// Sort options surfaced by the list's sort control. `next` is the default and
// mirrors the natural ordering produced by `compareAutomations` (active first,
// then soonest next run). Labels reuse the existing table column keys.
export const AUTOMATION_SORTS = [
  { value: "next", labelKey: "automations.table.nextRun" },
  { value: "last", labelKey: "automations.table.lastRun" },
  { value: "name", labelKey: "automations.table.name" },
  { value: "status", labelKey: "automations.table.status" },
];

// Stable rank for the "status" sort: live runs first, then ones needing review,
// then everything else.
function statusRank(automation) {
  if (automation?.has_running_run) return 0;
  if (automation?.has_failed_runs) return 1;
  if (isBrowserActive(automation)) return 2;
  return 3;
}

// Return a re-sorted copy of `automations` for the given sort key. Defaults to
// the natural next-run ordering so the control's default visibly matches what
// the list already does.
export function sortAutomations(automations, sort) {
  const list = [...automations];
  if (sort === "last") {
    return list.sort(
      (a, b) =>
        (b.last_run_timestamp ?? Number.NEGATIVE_INFINITY) -
        (a.last_run_timestamp ?? Number.NEGATIVE_INFINITY),
    );
  }
  if (sort === "name") {
    return list.sort((a, b) =>
      String(a.display_name).localeCompare(String(b.display_name)),
    );
  }
  if (sort === "status") {
    return list.sort(
      (a, b) =>
        statusRank(a) - statusRank(b) ||
        (nextRunTimestamp(a) ?? Number.MAX_SAFE_INTEGER) -
          (nextRunTimestamp(b) ?? Number.MAX_SAFE_INTEGER),
    );
  }
  return list.sort(compareAutomations);
}

export function normalizeAutomations(response, t, locale) {
  const automations = Array.isArray(response?.automations)
    ? response.automations
    : [];
  return automations
    .filter((automation) => SUPPORTED_SOURCE_TYPES.includes(automation?.source?.type))
    .map((automation) => normalizeAutomation(automation, t, locale))
    .sort(compareAutomations);
}

export function filterAutomations(automations, filter) {
  const strategy = AUTOMATION_FILTERS.find((item) => item.value === filter)?.predicate;
  return strategy ? automations.filter(strategy) : automations;
}

// Pick a glyph for an automation from its schedule cadence so each list row
// reads at a glance. Automations carry no icon in the wire payload, so the
// icon is derived: sub-hourly cadences feel like a live pulse, hour/day
// cadences read as a clock, and calendar-grained cadences (weekday / monthly /
// dated) read as a calendar. Anything we can't parse falls back to a bolt.
export function automationIcon(automation) {
  const cron = automation?.source?.cron;
  if (!cron || typeof cron !== "string") return "bolt";
  const parts = cronFields(cron);
  if (!parts) return "bolt";
  const { minute, hour, dayOfMonth, month, dayOfWeek } = parts;
  const everyDate = dayOfMonth === "*" && month === "*" && dayOfWeek === "*";
  if (everyDate && hour === "*") return "pulse"; // every minute / */N minutes
  if (everyDate && minute !== "*" && hour === "*") return "pulse";
  const normalizedDow = normalizeDayOfWeek(dayOfWeek);
  if (normalizedDow !== "*" || !everyDate) return "calendar"; // weekday / dated / monthly
  return "clock"; // daily / hourly at a fixed time
}

export function automationSummary(automations) {
  // Exclude completed (soft-completed one-shots) from summary cards so that
  // fetching with include_completed=true does not inflate the counts shown on
  // all other tabs.
  const visible = automations.filter((a) => a.state !== "completed");
  const active = visible.filter((automation) => isBrowserActive(automation)).length;
  // Count automations (not individual runs) so each card matches the
  // same-named filter tab, which filters automations via has_running_run /
  // has_failed_runs.
  const running = visible.filter((automation) => automation.has_running_run).length;
  const failures = visible.filter((automation) => automation.has_failed_runs).length;
  // Only automations that will actually fire contribute to "soonest next run".
  // Paused triggers keep their stored next_run_at slot, but they won't run, so
  // surfacing their time here would imply a run that never happens.
  const next = visible
    .filter(
      (automation) =>
        isBrowserActive(automation) && nextRunTimestamp(automation) != null,
    )
    .sort(
      (a, b) =>
        (a.next_run_timestamp ?? Number.MAX_SAFE_INTEGER) -
        (b.next_run_timestamp ?? Number.MAX_SAFE_INTEGER),
    )[0];
  return {
    scheduled: visible.length,
    active,
    running,
    failures,
    nextRun: next?.next_run_label || null,
  };
}

// Render a cron expression as a friendly, localized cadence string.
//
// The sentence templates ("Every day at {time}", "Weekdays at {time}", …) and
// the clock-free cadence words come from i18n keys (`t`), while the locale-
// grammar-heavy pieces — clock time, weekday name, month/day — are formatted
// with `Intl.DateTimeFormat` for `locale` so we don't hand-maintain weekday and
// month tables in every pack. Timezone, when known, is appended as a neutral
// parenthetical (omitted for minute/hour cadences where it is meaningless).
// Epoch-ms timestamp of the soonest run across all browser-active automations,
// or null when nothing is scheduled to fire. Mirrors the selection in
// `automationSummary` (active-only, parseable next_run_at) but returns the raw
// timestamp so the summary card can render a live countdown. Kept separate from
// `automationSummary` so that function's wire shape — asserted elsewhere —
// stays stable.
export function soonestNextRunAt(automations) {
  const next = automations
    .filter(
      (automation) =>
        isBrowserActive(automation) && nextRunTimestamp(automation) != null,
    )
    .sort(
      (a, b) =>
        (nextRunTimestamp(a) ?? Number.MAX_SAFE_INTEGER) -
        (nextRunTimestamp(b) ?? Number.MAX_SAFE_INTEGER),
    )[0];
  return next ? nextRunTimestamp(next) : null;
}

export function scheduleLabel(cron, timezone, t, locale) {
  const tr = typeof t === "function" ? t : (key) => key;
  if (!cron || typeof cron !== "string") return tr("automations.schedule.custom");
  const parts = cronFields(cron);
  if (!parts) return tr("automations.schedule.custom");

  const { minute, hour, dayOfMonth, month, dayOfWeek, year } = parts;

  const tz = timezone && typeof timezone === "string" ? timezone : null;
  const tzSuffix = tz ? ` (${tz})` : "";
  const everyDate =
    year === "*" && dayOfMonth === "*" && month === "*" && dayOfWeek === "*";

  // Sub-hourly / hourly cadences, where hour (and possibly minute) is a
  // wildcard or step and therefore has no single clock time. Timezone is
  // irrelevant for a minute-of-hour cadence, so it is omitted here. A `*/1`
  // step is the same as "every minute".
  if (everyDate && hour === "*") {
    if (minute === "*") return tr("automations.schedule.everyMinute");
    const step = minuteStep(minute);
    if (step === 1) return tr("automations.schedule.everyMinute");
    if (step) return tr("automations.schedule.everyMinutes", { count: step });
    if (isSingleNumber(minute, 0, 59)) {
      return tr("automations.schedule.hourlyAt", {
        minute: String(Number(minute)).padStart(2, "0"),
      });
    }
  }

  const time = formatCronTime(hour, minute, locale);
  if (!time) return tr("automations.schedule.custom");

  if (everyDate) {
    return tr("automations.schedule.everyDayAt", { time }) + tzSuffix;
  }
  const normalizedDayOfWeek = normalizeDayOfWeek(dayOfWeek);

  if (year === "*" && dayOfMonth === "*" && month === "*" && normalizedDayOfWeek === "1-5") {
    return tr("automations.schedule.weekdaysAt", { time }) + tzSuffix;
  }
  if (
    year === "*" &&
    dayOfMonth === "*" &&
    month === "*" &&
    isSingleNumber(normalizedDayOfWeek, 0, 7)
  ) {
    const weekday = weekdayName(Number(normalizedDayOfWeek) % 7, locale);
    return tr("automations.schedule.weekdayAt", { weekday, time }) + tzSuffix;
  }
  if (
    year === "*" &&
    isSingleNumber(dayOfMonth, 1, 31) &&
    month === "*" &&
    dayOfWeek === "*"
  ) {
    return (
      tr("automations.schedule.monthlyAt", { day: Number(dayOfMonth), time }) + tzSuffix
    );
  }
  if (
    isSingleNumber(dayOfMonth, 1, 31) &&
    isSingleNumber(month, 1, 12) &&
    dayOfWeek === "*" &&
    (year === "*" || isSingleNumber(year, 1970, 9999))
  ) {
    const date = monthDayLabel(
      Number(month),
      Number(dayOfMonth),
      year === "*" ? null : Number(year),
      locale,
    );
    return tr("automations.schedule.dateAt", { date, time }) + tzSuffix;
  }

  return tr("automations.schedule.custom");
}

// `fallback` is already-translated text the caller resolves via `t`; `locale`
// localizes the date itself so non-English users don't see English months.
// When `timezone` is a non-empty string it is forwarded to `Intl` so the wall
// clock reflects that timezone. The catch fallback deliberately omits timeZone
// (browser-local) — never substitute UTC.
export function formatAutomationDate(value, fallback = "Unknown", locale, timezone) {
  if (!value) return fallback;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return fallback;
  const tzOptions =
    timezone && typeof timezone === "string" ? { timeZone: timezone } : {};
  try {
    return date.toLocaleString(locale || [], {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      ...tzOptions,
    });
  } catch (_) {
    return date.toLocaleString([], {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }
}

// Locale-aware relative time ("2 hours ago", "in 5 minutes") via
// Intl.RelativeTimeFormat — no hand-maintained i18n strings. Returns null for
// missing/unparseable input so callers can omit the relative line entirely.
export function formatRelativeTime(value, locale) {
  if (!value) return null;
  // Older browsers / ICU-less Node test envs lack Intl.RelativeTimeFormat;
  // bail before constructing it so the catch can't re-throw and crash.
  if (typeof Intl === "undefined" || !Intl.RelativeTimeFormat) return null;
  const ts = typeof value === "number" ? value : Date.parse(value);
  if (Number.isNaN(ts)) return null;
  const diffMs = ts - Date.now();
  const absSec = Math.abs(diffMs) / 1000;
  const units = [
    ["year", 31536000],
    ["month", 2592000],
    ["week", 604800],
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
    ["second", 1],
  ];
  const [unit, secs] = units.find(([, s]) => absSec >= s) || ["second", 1];
  try {
    return new Intl.RelativeTimeFormat(locale || "en", { numeric: "auto" }).format(
      Math.round(diffMs / 1000 / secs),
      unit,
    );
  } catch (_) {
    return new Intl.RelativeTimeFormat("en", { numeric: "auto" }).format(
      Math.round(diffMs / 1000 / secs),
      unit,
    );
  }
}

export function stateLabel(state, t) {
  const key = STATE_PRESENTATION[state]?.labelKey || "automations.state.unknown";
  return tr(t)(key);
}

export function stateTone(state) {
  return STATE_PRESENTATION[state]?.tone || "muted";
}

export function primaryStatusLabel(automation, t) {
  if (isBrowserActive(automation) && automation?.has_running_run) {
    return tr(t)("automations.status.running");
  }
  if (isBrowserActive(automation) && automation?.has_failed_runs) {
    return tr(t)("automations.status.needsReview");
  }
  return stateLabel(automation?.state, t);
}

export function primaryStatusTone(automation) {
  if (isBrowserActive(automation) && automation?.has_running_run) return "info";
  if (isBrowserActive(automation) && automation?.has_failed_runs) return "danger";
  return stateTone(automation?.state);
}

export function lastStatusLabel(status, t) {
  const key = LAST_STATUS_PRESENTATION[status]?.labelKey || "automations.lastStatus.none";
  return tr(t)(key);
}

export function lastStatusTone(status) {
  return LAST_STATUS_PRESENTATION[status]?.tone || "muted";
}

export function runStatusLabel(status, t) {
  const key =
    RUN_STATUS_PRESENTATION[normalizeRunStatus(status)]?.labelKey ||
    "automations.runStatus.unknown";
  return tr(t)(key);
}

export function runStatusTone(status) {
  return RUN_STATUS_PRESENTATION[normalizeRunStatus(status)]?.tone || "muted";
}

// Format a one-shot trigger as "Once on <datetime> (<tz>)".
// Returns the custom-schedule fallback key when `at` is missing or unparseable.
function onceScheduleLabel(at, timezone, t, locale) {
  if (!at) return tr(t)("automations.schedule.custom");
  const datetime = formatAutomationDate(at, null, locale, timezone);
  if (!datetime) return tr(t)("automations.schedule.custom");
  const tzSuffix = timezone && typeof timezone === "string" ? ` (${timezone})` : "";
  return tr(t)("automations.schedule.onceAt", { datetime }) + tzSuffix;
}

// Dispatcher for the discriminated source union. A future source kind is a
// one-line addition in SUPPORTED_SOURCE_TYPES + a branch here.
function automationScheduleLabel(source, t, locale) {
  if (source?.type === "once") {
    return onceScheduleLabel(source.at, source.timezone, t, locale);
  }
  if (source?.type === "schedule") {
    // Preserve the pre-existing "UTC" default for schedule sources so that a
    // recurring trigger with no stored timezone still appends "(UTC)".
    return scheduleLabel(source.cron, source.timezone || "UTC", t, locale);
  }
  return tr(t)("automations.schedule.custom");
}

function normalizeAutomation(automation, t, locale) {
  const tx = tr(t);
  const recentRuns = normalizeRuns(automation.recent_runs, t, locale);
  const latestRun = recentRuns[0] || null;
  const currentRun = recentRuns.find((run) => run.status === "running") || null;
  const lastCompletedRun =
    recentRuns.find((run) => run.status === "ok" || run.status === "error") ||
    null;
  const lastStatus = lastCompletedRun?.status || automation.last_status;
  const lastRunAt = lastCompletedRun?.completed_at || automation.last_run_at || null;
  const normalized = {
    ...automation,
    recent_runs: recentRuns,
    has_running_run: recentRuns.some((run) => run.status === "running"),
    has_failed_runs: recentRuns.some((run) => run.status === "error"),
  };

  return {
    ...normalized,
    display_name: automation.name || tx("automations.untitled"),
    icon: automationIcon(automation),
    schedule_timezone: automation.source?.timezone || "UTC",
    schedule_label: automationScheduleLabel(automation.source, t, locale),
    state_label: stateLabel(automation.state, t),
    state_tone: stateTone(automation.state),
    primary_status_label: primaryStatusLabel(normalized, t),
    primary_status_tone: primaryStatusTone(normalized),
    next_run_timestamp: parseTimestamp(automation.next_run_at),
    next_run_label: formatAutomationDate(
      automation.next_run_at,
      tx("automations.date.notScheduled"),
      locale,
    ),
    last_run_label: formatAutomationDate(lastRunAt, tx("automations.date.noRuns"), locale),
    last_run_relative: formatRelativeTime(lastRunAt, locale),
    last_run_timestamp: parseTimestamp(lastRunAt),
    last_status_label: lastStatusLabel(lastStatus, t),
    last_status_tone: lastStatusTone(lastStatus),
    created_label: formatAutomationDate(
      automation.created_at,
      tx("automations.date.unknown"),
      locale,
    ),
    latest_run: latestRun,
    current_run: currentRun,
    success_rate_label: successRateLabel(recentRuns, t),
  };
}

function normalizeRuns(runs, t, locale) {
  const tx = tr(t);
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
        status_label: runStatusLabel(status, t),
        status_tone: runStatusTone(status),
        timestamp,
        timestamp_source: timestampSource,
        fired_label: formatAutomationDate(timestampSource, tx("automations.date.unscheduled"), locale),
        submitted_label: formatAutomationDate(run?.submitted_at, tx("automations.date.notSubmitted"), locale),
        completed_label: formatAutomationDate(run?.completed_at, tx("automations.date.notCompleted"), locale),
        // Only emit chat_path when a canonical thread_id is present. The backend sets
        // thread_id only after fire acceptance; pre-acceptance and pre-submit-failure rows
        // carry null/absent thread_id, which is falsy and suppresses the link.
        chat_path: run?.thread_id
          ? `/chat/${encodeURIComponent(run.thread_id)}`
          : null,
      };
    })
    .sort((a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0));
}

function normalizeRunStatus(status) {
  if (status === "ok" || status === "error" || status === "running") return status;
  return "unknown";
}

// Count recent runs by status so the UI can show a "how many" summary instead
// of relying on the reader counting densely-packed status dots. The API can
// return up to 25 recent runs while the dot strip only has room for a handful,
// so callers use `total` to render an overflow indicator (#4988).
export function summarizeRuns(runs) {
  const list = Array.isArray(runs) ? runs : [];
  const counts = { total: list.length, ok: 0, error: 0, running: 0, unknown: 0 };
  for (const run of list) {
    counts[normalizeRunStatus(run?.status)] += 1;
  }
  return counts;
}

// Solid fill classes for the proportion bar + legend dots, paired with the
// text tones above so the bar, the dots, and the chip labels all read as the
// same status colour.
const RUN_BAR_TONE = {
  ok: "bg-emerald-400",
  error: "bg-red-400",
  running: "bg-sky-400",
  unknown: "bg-iron-500",
};

// Ordered, non-empty status buckets for the recent-run summary chips. Kept in
// the presenter (not inline in the component) so a caller-level test can assert
// that no counted status — including `unknown` — is dropped from what the UI
// renders. Each entry carries the i18n key suffix, the chip text tone, and the
// solid fill class shared by the proportion bar and the legend dots.
export function runStatusBreakdown(runs) {
  const counts = summarizeRuns(runs);
  return [
    { key: "ok", tone: "text-emerald-300", barClass: RUN_BAR_TONE.ok, count: counts.ok },
    { key: "error", tone: "text-red-300", barClass: RUN_BAR_TONE.error, count: counts.error },
    { key: "running", tone: "text-sky-300", barClass: RUN_BAR_TONE.running, count: counts.running },
    { key: "unknown", tone: "text-iron-400", barClass: RUN_BAR_TONE.unknown, count: counts.unknown },
  ].filter((part) => part.count > 0);
}

// Percentage of terminal (ok + error) runs that succeeded, rounded. Returns
// null when nothing has completed yet so the UI can omit the figure rather than
// show a misleading 0%.
export function runSuccessRate(runs) {
  const counts = summarizeRuns(runs);
  const terminal = counts.ok + counts.error;
  if (!terminal) return null;
  return Math.round((counts.ok / terminal) * 100);
}

// The fully-resolved data the recent-run summary renders: the total label plus
// one chip per non-empty status (localized text + tone). `RunHistorySummary`
// maps this 1:1 with no logic of its own, so this function is the single place
// a status bucket could be dropped — and the place the caller-level test drives.
export function runSummaryView(runs, t) {
  const tx = tr(t);
  const counts = summarizeRuns(runs);
  const chips = runStatusBreakdown(runs).map((part) => ({
    ...part,
    text: tx(`automations.runs.${part.key}`, { count: part.count }),
  }));
  return {
    total: counts.total,
    totalText: tx("automations.runs.total", { count: counts.total }),
    successRate: runSuccessRate(runs),
    successRateText:
      runSuccessRate(runs) == null
        ? null
        : tx("automations.successRate.visible", { percent: runSuccessRate(runs) }),
    // The bar reuses the same ordered, non-empty buckets as the chips so the
    // proportions, legend dots, and labels can never disagree.
    segments: chips,
    chips,
  };
}

function successRateLabel(runs, t) {
  const tx = tr(t);
  const terminalRuns = runs.filter((run) => run.status === "ok" || run.status === "error");
  if (!terminalRuns.length) return tx("automations.successRate.none");
  const ok = terminalRuns.filter((run) => run.status === "ok").length;
  return tx("automations.successRate.visible", {
    percent: Math.round((ok / terminalRuns.length) * 100),
  });
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

function isBrowserCompleted(automation) {
  return automation?.state === "completed";
}

function nextRunTimestamp(automation) {
  return automation?.next_run_timestamp ?? parseTimestamp(automation?.next_run_at);
}

// Format a cron hour/minute as a locale-aware clock time (e.g. "9:00 AM" in
// en, "09:00" in de). Returns null for non-numeric fields so the caller falls
// back to "Custom schedule". The Date is built in local time and rendered
// without a timeZone option, so the displayed h:m is exactly what we put in —
// independent of the machine's timezone.
function intlDateTime(locale, options, date) {
  try {
    return new Intl.DateTimeFormat(locale || "en", options).format(date);
  } catch (_) {
    return new Intl.DateTimeFormat("en", options).format(date);
  }
}

function formatCronTime(hour, minute, locale) {
  if (!isSingleNumber(hour, 0, 23) || !isSingleNumber(minute, 0, 59)) return null;
  return intlDateTime(
    locale,
    { hour: "numeric", minute: "2-digit" },
    new Date(2001, 0, 1, Number(hour), Number(minute)),
  );
}

// Localized full weekday name for a cron day-of-week (0 = Sunday). Jan 7 2001
// was a Sunday, so offsetting from it yields the requested weekday.
function weekdayName(dayOfWeek, locale) {
  return intlDateTime(locale, { weekday: "long" }, new Date(2001, 0, 7 + dayOfWeek));
}

// Localized "month day" (and optional year), e.g. "Jan 1" / "Jan 1, 2027".
// The placeholder year for a yearless cron must be a leap year so that
// "Feb 29" (cron `0 0 29 2 *`) doesn't roll over to "Mar 1".
function monthDayLabel(month, day, year, locale) {
  const options =
    year != null
      ? { month: "short", day: "numeric", year: "numeric" }
      : { month: "short", day: "numeric" };
  return intlDateTime(locale, options, new Date(year != null ? year : 2000, month - 1, day));
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

// Parse a `*/N` step expression into N, returning null when it isn't a valid
// minute step (1..=59).
function minuteStep(value) {
  const match = /^\*\/(\d+)$/.exec(value);
  if (!match) return null;
  const step = Number(match[1]);
  return step >= 1 && step <= 59 ? step : null;
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
