import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import { runSummaryView } from "../lib/automations-presenters.js";
import { buildScopedLogsPath } from "../../logs/lib/logs-data.js";

const MAX_VISIBLE_DOTS = 8;

export function recentRunKey(run) {
  return run.run_id || run.thread_id || run.submitted_at || run.timestamp_source;
}

// A row of status dots for the most recent runs, capped at `MAX_VISIBLE_DOTS`.
// When more runs exist than fit, an overflow chip ("+N") makes the hidden count
// explicit instead of silently dropping runs off the end (#4988). Each dot
// keeps a hover tooltip describing its status and fire time.
export function RunDots({ runs = [] }) {
  const t = useT();
  const visibleRuns = runs.slice(0, MAX_VISIBLE_DOTS);
  if (!visibleRuns.length) {
    return html`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;
  }
  const overflow = runs.length - visibleRuns.length;

  return html`
    <div
      className="flex items-center gap-1"
      aria-label=${t("automations.runs.showingOf", { shown: visibleRuns.length, total: runs.length })}
    >
      ${visibleRuns.map((run) => html`
        <span
          key=${recentRunKey(run)}
          title=${`${run.status_label} · ${run.fired_label}`}
          className=${cn(
            "h-2.5 w-2.5 rounded-full ring-1 ring-inset",
            run.status === "ok" && "bg-emerald-400 ring-emerald-300/40",
            run.status === "error" && "bg-red-400 ring-red-300/40",
            run.status === "running" &&
              "bg-sky-400 ring-sky-300/50 animate-[v2-breathe_2s_ease-in-out_infinite]",
            run.status === "unknown" && "bg-iron-600 ring-iron-500/40"
          )}
        />
      `)}
      ${overflow > 0 &&
      html`<span
        className="ml-1 inline-flex h-5 items-center rounded-full bg-[var(--v2-surface-soft)] px-1.5 font-mono text-[10px] tabular-nums text-iron-400"
        title=${t("automations.runs.showingOf", { shown: visibleRuns.length, total: runs.length })}
      >
        +${overflow}
      </span>`}
    </div>
  `;
}

// Aggregate view of recent-run health: total + success rate, a proportional
// fill bar, and a dot-led legend of per-status counts. Replaces the old plain
// "12 runs · 9 OK · 2 failed" text line with something that reads at a glance
// while keeping every counted bucket visible (#4988). All chip/segment/bucket
// decisions live in runSummaryView (pure + tested); this component only maps
// the resolved view to markup.
export function RunHistorySummary({ runs = [], className = "" }) {
  const t = useT();
  const view = runSummaryView(runs, t);
  if (!view.total) {
    return html`<span className=${cn("text-[11px] text-iron-400", className)}>
      ${t("automations.table.noRuns")}
    </span>`;
  }

  const rateTone =
    view.successRate == null
      ? "text-iron-300"
      : view.successRate >= 90
        ? "text-emerald-300"
        : view.successRate >= 50
          ? "text-amber-300"
          : "text-red-300";

  return html`
    <div className=${cn("w-full max-w-[15rem]", className)}>
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-[11px] text-iron-300">${view.totalText}</span>
        ${view.successRate != null &&
        html`<span
          className=${cn("text-[11px] font-semibold tabular-nums", rateTone)}
          title=${view.successRateText}
        >
          ${view.successRate}%
        </span>`}
      </div>

      <div
        className="mt-1.5 flex h-2 w-full overflow-hidden rounded-full bg-[var(--v2-surface-muted)]"
        role="img"
        aria-label=${view.successRateText || view.totalText}
      >
        ${view.segments.map(
          (segment) => html`<div
            key=${segment.key}
            className=${cn("h-full", segment.barClass)}
            style=${{ flexGrow: segment.count }}
            title=${segment.text}
          />`
        )}
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1">
        ${view.chips.map(
          (chip) => html`<span
            key=${chip.key}
            className="inline-flex items-center gap-1.5 text-[11px] text-iron-300"
          >
            <span className=${cn("h-1.5 w-1.5 rounded-full", chip.barClass)} />
            ${chip.text}
          </span>`
        )}
      </div>
    </div>
  `;
}

export function RecentRunRow({ run, onOpenRun, onOpenLogs }) {
  const t = useT();
  const canOpen = Boolean(run.chat_path);
  const logsPath = buildScopedLogsPath({
    threadId: run.thread_id,
    runId: run.run_id,
  });
  const canOpenLogs = Boolean((run.thread_id || run.run_id) && onOpenLogs);

  return html`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${StatusPill} tone=${run.status_tone} label=${run.status_label} />
      </div>
      <div className="min-w-0">
        <div className="text-sm font-semibold text-iron-100">${run.fired_label}</div>
        <div className="mt-1 truncate font-mono text-[11px] text-iron-400">
          ${run.thread_id
            ? `${t("automations.detail.thread")} ${run.thread_id}`
            : t("automations.detail.noThread")}
        </div>
        ${run.run_id &&
        html`
          <div className="mt-1 truncate font-mono text-[11px] text-iron-500">
            ${t("automations.detail.run")} ${run.run_id}
          </div>
        `}
      </div>
      <div className="flex flex-wrap items-center gap-2 sm:justify-end">
        <${Button}
          variant="secondary"
          size="sm"
          disabled=${!canOpen}
          onClick=${canOpen ? () => onOpenRun(run.chat_path) : undefined}
        >
          <${Icon} name="chat" className="mr-1.5 h-4 w-4" />
          ${t("automations.detail.openRun")}
        <//>
        <${Button}
          variant="ghost"
          size="sm"
          disabled=${!canOpenLogs}
          onClick=${canOpenLogs ? () => onOpenLogs(logsPath) : undefined}
        >
          <${Icon} name="file" className="mr-1.5 h-4 w-4" />
          ${t("nav.logs")}
        <//>
      </div>
    </div>
  `;
}
