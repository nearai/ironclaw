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
      className="flex items-center gap-1.5"
      aria-label=${t("automations.runs.showingOf", { shown: visibleRuns.length, total: runs.length })}
    >
      ${visibleRuns.map((run) => html`
        <span
          key=${recentRunKey(run)}
          title=${`${run.status_label} · ${run.fired_label}`}
          className=${cn(
            "h-3 w-3 rounded-full border",
            run.status === "ok" && "border-emerald-300/50 bg-emerald-400",
            run.status === "error" && "border-red-300/50 bg-red-400",
            run.status === "running" && "border-sky-300/60 bg-sky-400",
            run.status === "unknown" && "border-iron-500 bg-iron-600"
          )}
        />
      `)}
      ${overflow > 0 &&
      html`<span
        className="ml-0.5 font-mono text-[11px] text-iron-400"
        title=${t("automations.runs.showingOf", { shown: visibleRuns.length, total: runs.length })}
      >
        +${overflow}
      </span>`}
    </div>
  `;
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
    return html`<span className=${cn("text-[11px] text-iron-400", className)}>
      ${t("automations.table.noRuns")}
    </span>`;
  }

  return html`
    <div className=${cn("flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]", className)}>
      <span className="text-iron-300">${view.totalText}</span>
      ${view.chips.map(
        (chip) => html`<span key=${chip.key} className=${chip.tone}>${chip.text}</span>`
      )}
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
