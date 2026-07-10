// @ts-nocheck
import React from "react";
import { Card } from "../../../design-system/card";
import { Badge } from "../../../design-system/primitives";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";

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

// Read-only stat cards summarising the list. Each cell is a proper DS Card
// (default variant, small radius) so the strip shares surface, radius, and
// shadow semantics with every other card in the app. The grid reflows
// 2 → 3 → 5 columns as the viewport grows; the next-run card spans the full
// row on the 2-column layout so the countdown gets room to breathe.
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
      tone: "signal",
      badgeLabel: t("automations.badge.signal"),
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
      span: "col-span-2 md:col-span-1",
    },
  ];

  return (
    <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-5">
      {cells.map((cell) => (
        <Card
          key={cell.key}
          variant="flat"
          radius="sm"
          className={cn("flex min-w-0 flex-col p-4", cell.span)}
        >
          <div className="flex items-center justify-between gap-2">
            <span
              className="truncate font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
              title={cell.label}
            >
              {cell.label}
            </span>
            {/* The tone chip is decorative context; below sm it would crowd
                the label into truncation, so it steps aside. */}
            <span className="hidden shrink-0 sm:block">
              <Badge tone={cell.tone} label={cell.badgeLabel} size="sm" />
            </span>
          </div>
          <div
            className="mt-2.5 truncate text-[1.5rem] font-semibold leading-none tracking-[-0.02em] tabular-nums text-[var(--v2-text-strong)]"
            title={String(cell.value)}
          >
            {cell.value}
          </div>
          <div
            className="mt-1.5 overflow-hidden text-xs leading-snug text-[var(--v2-text-muted)] [display:-webkit-box] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
            title={cell.detail}
          >
            {cell.detail}
          </div>
        </Card>
      ))}
    </div>
  );
}
