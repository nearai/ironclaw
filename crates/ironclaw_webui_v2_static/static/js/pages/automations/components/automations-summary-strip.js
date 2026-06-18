import { Badge, Panel } from "../../../design-system/primitives.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";

// Re-render once a second so the next-run countdown ticks. A single shared
// timer for the whole strip (not one per card) keeps it cheap.
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

function SummaryCard({ card, activeFilter, onSelectFilter }) {
  const t = useT();
  const interactive = Boolean(card.filter && onSelectFilter);
  const isActive = interactive && activeFilter === card.filter;

  const inner = html`
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
          ${card.label}
        </span>
        <${Badge} tone=${card.tone} label=${card.badgeLabel} size="sm" />
      </div>
      <div
        className=${cn(
          "mt-2.5 truncate font-medium tracking-[-0.03em] tabular-nums text-iron-100",
          card.valueClassName || "text-[1.6rem]"
        )}
      >
        ${card.value}
      </div>
      <div
        className="mt-auto truncate pt-1 text-[13px] leading-[1.2] text-iron-300"
        title=${card.detail}
      >
        ${card.detail}
      </div>
    </div>
  `;

  const baseClass =
    "flex h-full flex-col rounded-[14px] border border-white/8 bg-white/[0.03] px-3.5 py-3 text-left";

  if (!interactive) {
    return html`<div className=${baseClass}>${inner}</div>`;
  }
  return html`
    <button
      type="button"
      aria-pressed=${isActive}
      title=${t("automations.summary.filterAction", { label: card.label })}
      onClick=${() => onSelectFilter(card.filter)}
      className=${cn(
        baseClass,
        "transition-colors hover:border-white/20 hover:bg-white/[0.05]",
        "focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",
        isActive && "border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30"
      )}
    >
      ${inner}
    </button>
  `;
}

export function AutomationsSummaryStrip({ summary, nextRunAt, activeFilter, onSelectFilter }) {
  const t = useT();
  const now = useNow();

  // Next-run card: a live countdown as the headline with the absolute time as
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

  const cards = [
    {
      key: "scheduled",
      label: t("automations.summary.scheduled"),
      value: summary?.scheduled ?? 0,
      tone: "muted",
      badgeLabel: t("automations.badge.muted"),
      detail: t("automations.summary.scheduledDetail"),
      filter: "all",
    },
    {
      key: "active",
      label: t("automations.summary.active"),
      value: summary?.active ?? 0,
      tone: "signal",
      badgeLabel: t("automations.badge.signal"),
      detail: t("automations.summary.activeDetail"),
      filter: "active",
    },
    {
      key: "running",
      label: t("automations.summary.running"),
      value: summary?.running ?? 0,
      tone: "info",
      badgeLabel: t("automations.badge.info"),
      detail: t("automations.summary.runningDetail"),
      filter: "running",
    },
    {
      key: "failures",
      label: t("automations.summary.failures"),
      value: summary?.failures ?? 0,
      tone: (summary?.failures ?? 0) > 0 ? "danger" : "success",
      badgeLabel:
        (summary?.failures ?? 0) > 0
          ? t("automations.badge.danger")
          : t("automations.badge.success"),
      detail: t("automations.summary.failuresDetail"),
      filter: (summary?.failures ?? 0) > 0 ? "failures" : null,
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
    <${Panel} className="p-3 sm:p-3.5">
      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-3 xl:grid-cols-5">
        ${cards.map(
          (card) => html`<${SummaryCard}
            key=${card.key}
            card=${card}
            activeFilter=${activeFilter}
            onSelectFilter=${onSelectFilter}
          />`
        )}
      </div>
    <//>
  `;
}
