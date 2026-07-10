import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
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
  const list = Array.isArray(runs) ? runs : [];
  const visibleRuns = list.slice(0, MAX_VISIBLE_DOTS);
  if (!visibleRuns.length) {
    return html`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;
  }
  const overflow = list.length - visibleRuns.length;
  const overflowLabel = `+${Math.min(overflow, 999)}`;

  return html`
    <div
      className="flex items-center gap-1"
      aria-label=${t("automations.runs.showingOf", { shown: visibleRuns.length, total: list.length })}
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
              // they fade to a translucent fill behind a coloured ring. Colours
              // come from the canonical status tokens so the dots match the
              // proportion bar, row status text, and Badge pills exactly.
              run.status === "ok" &&
                (stale
                  ? "bg-[color-mix(in_srgb,var(--v2-positive-text)_20%,transparent)] ring-[color-mix(in_srgb,var(--v2-positive-text)_50%,transparent)]"
                  : "bg-[var(--v2-positive-text)] ring-[color-mix(in_srgb,var(--v2-positive-text)_40%,transparent)]"),
              run.status === "error" &&
                (stale
                  ? "bg-[color-mix(in_srgb,var(--v2-danger-text)_20%,transparent)] ring-[color-mix(in_srgb,var(--v2-danger-text)_50%,transparent)]"
                  : "bg-[var(--v2-danger-text)] ring-[color-mix(in_srgb,var(--v2-danger-text)_40%,transparent)]"),
              // In-progress runs oscillate their opacity so live work is obvious.
              run.status === "running" &&
                "bg-[var(--v2-info-text)] ring-[color-mix(in_srgb,var(--v2-info-text)_60%,transparent)] animate-pulse",
              run.status === "unknown" &&
                "bg-[var(--v2-text-faint)] ring-[color-mix(in_srgb,var(--v2-text-faint)_40%,transparent)]",
              stale && "opacity-50"
            )}
          />
        `;
      })}
      ${overflow > 0 &&
      html`<span
        className="ml-1 inline-flex h-5 items-center rounded-full bg-[var(--v2-surface-soft)] px-1.5 font-mono text-[10px] tabular-nums text-iron-400"
        title=${t("automations.runs.showingOf", { shown: visibleRuns.length, total: list.length })}
      >
        ${overflowLabel}
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
      ? "text-[var(--v2-text-muted)]"
      : view.successRate >= 90
        ? "text-[var(--v2-positive-text)]"
        : view.successRate >= 50
          ? "text-[var(--v2-warning-text)]"
          : "text-[var(--v2-danger-text)]";

  return html`
    <div
      className=${cn(
        "w-full rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4",
        className
      )}
    >
      <div className="flex items-baseline justify-between gap-3">
        <div className="flex items-baseline gap-1.5">
          <span className="text-lg font-semibold tabular-nums leading-none text-iron-100">
            ${view.total}
          </span>
          <span className="text-xs font-medium text-[var(--v2-text-muted)]">
            ${t("automations.runs.countLabel")}
          </span>
        </div>
        ${view.successRate != null &&
        html`<div className="flex items-baseline gap-1.5">
          <span
            className=${cn("text-lg font-semibold tabular-nums leading-none", rateTone)}
            title=${view.successRateText}
          >
            ${view.successRate}%
          </span>
          <span className="text-xs font-medium text-[var(--v2-text-muted)]">
            ${t("automations.detail.successRate")}
          </span>
        </div>`}
      </div>

      <div
        className="mt-3 flex h-2 w-full overflow-hidden rounded-full bg-[var(--v2-surface-muted)]"
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

      <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1.5">
        ${view.chips.map(
          (chip) => html`<span
            key=${chip.key}
            className="inline-flex items-center gap-1.5 text-xs font-medium text-[var(--v2-text)]"
          >
            <span
              aria-hidden="true"
              className=${cn("h-2 w-2 shrink-0 rounded-full", chip.barClass)}
            />
            ${chip.text}
          </span>`
        )}
      </div>
    </div>
  `;
}

// Status rendered as a plain dot + text (no pill chrome) so the run list
// reads as data, not as a wall of badges. Tone maps to the semantic ramp.
const RUN_STATUS_TEXT = {
  success: "text-[var(--v2-positive-text)]",
  danger: "text-[var(--v2-danger-text)]",
  info: "text-[var(--v2-info-text)]",
  muted: "text-[var(--v2-text-muted)]",
};
const RUN_STATUS_DOT = {
  success: "bg-[var(--v2-positive-text)]",
  danger: "bg-[var(--v2-danger-text)]",
  info: "bg-[var(--v2-info-text)]",
  muted: "bg-[var(--v2-text-faint)]",
};

export function RecentRunRow({ run, onOpenRun, onOpenLogs }) {
  const t = useT();
  const canOpen = Boolean(run.chat_path);
  const logsPath = buildScopedLogsPath({
    threadId: run.thread_id,
    runId: run.run_id,
  });
  const canOpenLogs = Boolean((run.thread_id || run.run_id) && onOpenLogs);
  const tone = RUN_STATUS_TEXT[run.status_tone] ? run.status_tone : "muted";

  return html`
    <div
      className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1.5 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[minmax(0,11rem)_minmax(0,1fr)_auto]"
    >
      <div className="min-w-0">
        <div className="text-sm font-semibold text-iron-100">${run.fired_label}</div>
        <div className="mt-1 flex items-center gap-1.5">
          <span
            aria-hidden="true"
            className=${cn("h-1.5 w-1.5 shrink-0 rounded-full", RUN_STATUS_DOT[tone])}
          />
          <span className=${cn("text-xs font-medium", RUN_STATUS_TEXT[tone])}>
            ${run.status_label}
          </span>
        </div>
      </div>
      <div className="order-last col-span-2 min-w-0 sm:order-none sm:col-span-1">
        <div className="truncate font-mono text-[11px] text-iron-400">
          ${run.thread_id
            ? `${t("automations.detail.thread")} ${run.thread_id}`
            : t("automations.detail.noThread")}
        </div>
        ${run.run_id &&
        html`
          <div className="mt-1 truncate font-mono text-[11px] text-[var(--v2-text-faint)]">
            ${t("automations.detail.run")} ${run.run_id}
          </div>
        `}
      </div>
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
          size="icon-sm"
          aria-label=${t("nav.logs")}
          title=${t("nav.logs")}
          disabled=${!canOpenLogs}
          onClick=${canOpenLogs ? () => onOpenLogs(logsPath) : undefined}
        >
          <${Icon} name="file" className="h-4 w-4" />
        <//>
      </div>
    </div>
  `;
}
