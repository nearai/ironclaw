import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import { runSummaryView } from "../lib/automations-presenters.js";
import { buildScopedLogsPath } from "../../logs/lib/logs-data.js";

const MAX_VISIBLE_DOTS = 8;
// Completed runs older than this fade back so recent activity reads first.
const STALE_RUN_MS = 24 * 60 * 60 * 1000;

function isStaleRun(run) {
  return (
    run.status !== "running" &&
    typeof run.timestamp === "number" &&
    Date.now() - run.timestamp > STALE_RUN_MS
  );
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
      ${visibleRuns.map((run) => {
        const stale = isStaleRun(run);
        return html`
          <span
            key=${recentRunKey(run)}
            title=${`${run.status_label} · ${run.fired_label}`}
            className=${cn(
              "h-2.5 w-2.5 rounded-full ring-1 ring-inset transition-opacity",
              // Recent completions read as a solid dot; once they age past a day
              // they fade to a translucent fill behind a coloured ring.
              run.status === "ok" &&
                (stale
                  ? "bg-emerald-400/20 ring-emerald-400/50"
                  : "bg-emerald-400 ring-emerald-300/40"),
              run.status === "error" &&
                (stale
                  ? "bg-red-400/20 ring-red-400/50"
                  : "bg-red-400 ring-red-300/40"),
              // In-progress runs oscillate their opacity so live work is obvious.
              run.status === "running" && "bg-sky-400 ring-sky-300/60 animate-pulse",
              run.status === "unknown" && "bg-iron-600 ring-iron-500/40",
              stale && "opacity-50"
            )}
          />
        `;
      })}
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
    <div
      className=${cn(
        "w-full rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",
        className
      )}
    >
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-sm font-medium text-iron-200">${view.totalText}</span>
        ${view.successRate != null &&
        html`<span
          className=${cn("text-sm font-semibold tabular-nums", rateTone)}
          title=${view.successRateText}
        >
          ${view.successRate}%
        </span>`}
      </div>

      <div
        className="mt-2.5 flex h-2.5 w-full overflow-hidden rounded-full bg-[var(--v2-surface-muted)]"
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

      <div className="mt-2.5 flex flex-wrap items-center gap-x-4 gap-y-1.5">
        ${view.chips.map(
          (chip) => html`<span
            key=${chip.key}
            className="inline-flex items-center gap-1.5 text-xs text-iron-300"
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
    <div className="flex flex-wrap items-center gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0">
      <div className="min-w-0 flex-1">
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
      <${StatusPill}
        tone=${run.status_tone}
        label=${run.status_label}
        className="shrink-0"
      />
      <div className="flex shrink-0 items-center gap-2">
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
