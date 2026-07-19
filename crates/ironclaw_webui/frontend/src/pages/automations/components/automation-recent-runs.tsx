import { Link } from "react-router";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { StatusPill } from "../../../design-system/primitives";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
import { runSummaryView } from "../lib/automations-presenters";
import { buildScopedLogsPath } from "../../logs/lib/logs-data";

const MAX_VISIBLE_DOTS = 8;
const MAX_NAVIGATION_ID_LENGTH = 512;
const DISABLED_LINK_TARGET = "#";

function navigationId(value) {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > MAX_NAVIGATION_ID_LENGTH || /[\u0000-\u001F\u007F]/.test(trimmed)) {
    return null;
  }
  return trimmed;
}

export function recentRunKey(run) {
  return run.run_id || run.thread_id || run.submitted_at || run.timestamp_source;
}

// A row of status dots for the most recent runs, capped at `MAX_VISIBLE_DOTS`.
// When more runs exist than fit, an overflow chip ("+N") makes the hidden count
// explicit instead of silently dropping runs off the end (#4988). Each dot
// keeps a hover tooltip describing its status and fire time.
export function RunDots({ runs = [] }) {
  const t = useT();
  const list = Array.isArray(runs) ? runs : [];
  const visibleRuns = list.slice(0, MAX_VISIBLE_DOTS);
  if (!visibleRuns.length) {
    return (<span className="text-xs text-iron-400">{t("automations.table.noRuns")}</span>);
  }
  const overflow = list.length - visibleRuns.length;
  const overflowLabel = `+${Math.min(overflow, 999)}`;

  return (
    <div
      className="flex items-center gap-1.5"
      aria-label={t("automations.runs.showingOf", { shown: visibleRuns.length, total: list.length })}
    >
      {visibleRuns.map((run) => (
        <span
          key={recentRunKey(run)}
          title={`${run.status_label} · ${run.fired_label}`}
          className={cn(
            "h-3 w-3 rounded-full border",
            run.status === "ok" &&
              "border-[color-mix(in_srgb,var(--v2-positive-text)_50%,var(--v2-panel-border))] bg-[var(--v2-positive-text)]",
            run.status === "error" && "border-red-300/50 bg-red-400",
            run.status === "running" && "border-sky-300/60 bg-sky-400",
            run.status === "unknown" &&
              "border-[var(--v2-text-faint)] bg-[var(--v2-text-muted)]"
          )}
        />
      ))}
      {overflow > 0 &&
      (<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title={t("automations.runs.showingOf", { shown: visibleRuns.length, total: list.length })}
      >
        {overflowLabel}
      </span>)}
    </div>
  );
}

// Compact textual breakdown of recent-run statuses ("12 runs · 9 OK · 2 failed
// · 1 running"). Zero-count categories are omitted. This is the "run count
// summary" the dot strip alone can't convey at a glance (#4988).
export function RunHistorySummary({ runs = [], className = "" }) {
  const t = useT();
  // All chip/text/bucket decisions live in runSummaryView (pure + tested); this
  // component only maps the resolved view to spans.
  const view = runSummaryView(runs, t);
  if (!view.total) {
    return (<span className={cn("text-[11px] text-iron-400", className)}>
      {t("automations.table.noRuns")}
    </span>);
  }

  return (
    <div className={cn("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]", className)}>
      <span className="text-iron-300">{view.totalText}</span>
      {view.chips.map(
        (chip) => (<span key={chip.key} className={chip.tone}>{chip.text}</span>)
      )}
    </div>
  );
}

export function RecentRunRow({ run }) {
  const t = useT();
  const threadId = navigationId(run.thread_id);
  const runId = navigationId(run.run_id);
  const chatPath = threadId ? `/chat/${encodeURIComponent(threadId)}` : null;
  const canOpenLogs = Boolean(threadId || runId);
  const logsPath = canOpenLogs
    ? buildScopedLogsPath({
        threadId,
        runId,
      })
    : DISABLED_LINK_TARGET;

  return (
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <StatusPill tone={run.status_tone} label={run.status_label} />
      </div>
      <div className="min-w-0">
        <div className="text-sm font-semibold text-iron-100">{run.fired_label}</div>
        <div className="mt-1 truncate font-mono text-[11px] text-iron-400">
          {run.thread_id
            ? `${t("automations.detail.thread")} ${run.thread_id}`
            : t("automations.detail.noThread")}
        </div>
        {run.run_id &&
        (
          <div className="mt-1 truncate font-mono text-[11px] text-[var(--v2-text-muted)]">
            {t("automations.detail.run")} {run.run_id}
          </div>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-2 sm:justify-end">
        <Button
          as={Link}
          to={chatPath || DISABLED_LINK_TARGET}
          variant="secondary"
          size="sm"
          disabled={!chatPath}
          data-testid="automation-run-open"
        >
          <Icon name="chat" className="mr-1.5 h-4 w-4" />
          {t("automations.detail.openRun")}
        </Button>
        <Button
          as={Link}
          to={logsPath}
          variant="ghost"
          size="sm"
          disabled={!canOpenLogs}
          data-testid="automation-run-logs"
        >
          <Icon name="file" className="mr-1.5 h-4 w-4" />
          {t("nav.logs")}
        </Button>
      </div>
    </div>
  );
}
