import { Badge } from "../../../design-system/primitives.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";

// Re-render once a second so the next-run countdown ticks. A single shared
// timer for the whole strip (not one per cell) keeps it cheap.
function useNow(intervalMs = 1000) {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

// Format a positive remaining duration as a compact countdown. Under an hour it
// ticks as m:ss (so the seconds animate seamlessly under tabular-nums); beyond
// that it steps down in the coarser unit that's actually changing. Returns null
// when the target is in the past so the caller can show "Due now".
function formatCountdown(ms) {
  if (ms <= 0) return null;
  const totalSec = Math.floor(ms / 1000);
  const days = Math.floor(totalSec / 86400);
  const hours = Math.floor((totalSec % 86400) / 3600);
  const minutes = Math.floor((totalSec % 3600) / 60);
  const seconds = totalSec % 60;
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, "0")}m`;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

// A static, read-only stats strip. Deliberately styled as a single recessed
// (inset) bar with hairline-separated cells so it reads as ambient context —
// distinct from the interactive filter pills that actually drive the list.
export function AutomationsSummaryStrip({ summary, nextRunAt }) {
  const t = useT();
  const now = useNow();

  // Next-run cell: a live countdown as the headline with the absolute time as
  // the sub-line. Falls back to "None" when nothing is scheduled to fire.
  const hasNextRun = typeof nextRunAt === "number" && Number.isFinite(nextRunAt);
  const countdown = hasNextRun ? formatCountdown(nextRunAt - now) : null;
  const nextRunValue = !hasNextRun
    ? t("automations.summary.none")
    : countdown == null
      ? t("automations.summary.nextRunDue")
      : countdown;
  const nextRunDetail = hasNextRun
    ? summary?.nextRun || t("automations.summary.nextRunDetail")
    : t("automations.summary.nextRunDetail");

  const failures = summary?.failures ?? 0;
  const cells = [
    {
      key: "scheduled",
      label: t("automations.summary.scheduled"),
      value: summary?.scheduled ?? 0,
      tone: "muted",
      badgeLabel: t("automations.badge.muted"),
      detail: t("automations.summary.scheduledDetail"),
    },
    {
      key: "active",
      label: t("automations.summary.active"),
      value: summary?.active ?? 0,
      tone: "signal",
      badgeLabel: t("automations.badge.signal"),
      detail: t("automations.summary.activeDetail"),
    },
    {
      key: "running",
      label: t("automations.summary.running"),
      value: summary?.running ?? 0,
      tone: "info",
      badgeLabel: t("automations.badge.info"),
      detail: t("automations.summary.runningDetail"),
    },
    {
      key: "failures",
      label: t("automations.summary.failures"),
      value: failures,
      tone: failures > 0 ? "danger" : "success",
      badgeLabel:
        failures > 0
          ? t("automations.badge.danger")
          : t("automations.badge.success"),
      detail: t("automations.summary.failuresDetail"),
    },
    {
      key: "nextRun",
      label: t("automations.summary.nextRun"),
      value: nextRunValue,
      tone: "info",
      badgeLabel: t("automations.badge.info"),
      detail: nextRunDetail,
    },
  ];

  return html`
    <div
      className="overflow-hidden rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)]"
    >
      <div
        className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 lg:divide-x lg:divide-[var(--v2-panel-border)]"
      >
        ${cells.map(
          (cell) => html`
            <div key=${cell.key} className="flex min-w-0 flex-col px-4 py-3">
              <div className="flex items-center justify-between gap-2">
                <span className="truncate font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
                  ${cell.label}
                </span>
                <${Badge} tone=${cell.tone} label=${cell.badgeLabel} size="sm" />
              </div>
              <div className="mt-1.5 truncate text-2xl font-medium tracking-[-0.03em] tabular-nums text-iron-100">
                ${cell.value}
              </div>
              <div
                className="mt-0.5 truncate text-xs leading-[1.3] text-iron-400"
                title=${cell.detail}
              >
                ${cell.detail}
              </div>
            </div>
          `
        )}
      </div>
    </div>
  `;
}
