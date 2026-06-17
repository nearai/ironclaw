import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Panel, StatCard } from "../../../design-system/primitives.js";
import { cn } from "../../../utils/cn.js";

export function AutomationsSummaryStrip({ summary, activeFilter, onSelectFilter }) {
  const t = useT();
  const cards = [
    {
      key: "scheduled",
      label: t("automations.summary.scheduled"),
      value: summary?.scheduled ?? 0,
      tone: "muted",
      detail: t("automations.summary.scheduledDetail"),
      filter: "all",
    },
    {
      key: "active",
      label: t("automations.summary.active"),
      value: summary?.active ?? 0,
      tone: "signal",
      detail: t("automations.summary.activeDetail"),
      filter: "active",
    },
    {
      key: "running",
      label: t("automations.summary.running"),
      value: summary?.running ?? 0,
      tone: "info",
      detail: t("automations.summary.runningDetail"),
      filter: "running",
    },
    {
      key: "failures",
      label: t("automations.summary.failures"),
      value: summary?.failures ?? 0,
      tone: (summary?.failures ?? 0) > 0 ? "danger" : "success",
      detail: t("automations.summary.failuresDetail"),
      // The failures card is the primary actionable card (#5004): clicking it
      // filters the list down to the automations with failed runs so the user
      // can jump straight to what went wrong instead of hunting through
      // history. Only offer the jump when there is at least one failure.
      filter: (summary?.failures ?? 0) > 0 ? "failures" : null,
    },
    {
      key: "nextRun",
      label: t("automations.summary.nextRun"),
      value: summary?.nextRun || t("automations.summary.none"),
      tone: "info",
      detail: t("automations.summary.nextRunDetail"),
      // NEXT RUN is a date string, not a count — use a smaller size so it isn't
      // truncated to "Jun…" inside a narrow card.
      valueClassName: "text-lg md:text-xl",
    },
  ];

  return html`
    <${Panel} className="p-4 sm:p-5">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        ${cards.map((card) => {
          const interactive = Boolean(card.filter && onSelectFilter);
          const isActive = interactive && activeFilter === card.filter;
          const inner = html`
            <${StatCard}
              label=${card.label}
              value=${card.value}
              tone=${card.tone}
              badgeLabel=${t(`automations.badge.${card.tone}`)}
              detail=${card.detail}
              valueClassName=${card.valueClassName}
              showDivider=${false}
              className="px-0 py-0"
            />
          `;
          const baseClass =
            "rounded-[14px] border border-white/8 bg-white/[0.03] p-4 text-left";
          if (!interactive) {
            return html`<div key=${card.key} className=${baseClass}>${inner}</div>`;
          }
          return html`
            <button
              key=${card.key}
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
        })}
      </div>
    <//>
  `;
}
