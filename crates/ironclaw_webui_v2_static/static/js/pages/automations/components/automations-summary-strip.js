import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Panel, StatCard } from "../../../design-system/primitives.js";

export function AutomationsSummaryStrip({ summary }) {
  const t = useT();
  const cards = [
    {
      key: "scheduled",
      label: t("automations.summary.scheduled"),
      value: summary?.scheduled ?? 0,
      tone: "muted",
      detail: t("automations.summary.scheduledDetail"),
    },
    {
      key: "active",
      label: t("automations.summary.active"),
      value: summary?.active ?? 0,
      tone: "signal",
      detail: t("automations.summary.activeDetail"),
    },
    {
      key: "running",
      label: t("automations.summary.running"),
      value: summary?.running ?? 0,
      tone: "info",
      detail: t("automations.summary.runningDetail"),
    },
    {
      key: "failures",
      label: t("automations.summary.failures"),
      value: summary?.failures ?? 0,
      tone: (summary?.failures ?? 0) > 0 ? "danger" : "success",
      detail: t("automations.summary.failuresDetail"),
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
        ${cards.map((card) => html`
          <div
            key=${card.key}
            className="rounded-[14px] border border-white/8 bg-white/[0.03] p-4"
          >
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
          </div>
        `)}
      </div>
    <//>
  `;
}
