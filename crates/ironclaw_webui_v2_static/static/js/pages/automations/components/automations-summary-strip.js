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
      key: "paused",
      label: t("automations.summary.paused"),
      value: summary?.paused ?? 0,
      tone: "warning",
      detail: t("automations.summary.pausedDetail"),
    },
    {
      key: "nextRun",
      label: t("automations.summary.nextRun"),
      value: summary?.nextRun || t("automations.summary.none"),
      tone: "info",
      detail: t("automations.summary.nextRunDetail"),
    },
  ];

  return html`
    <${Panel} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${cards.map((card) => html`
          <div
            key=${card.key}
            className="rounded-[14px] border border-white/8 bg-white/[0.03] p-4"
          >
            <${StatCard}
              label=${card.label}
              value=${card.value}
              tone=${card.tone}
              detail=${card.detail}
              showDivider=${false}
              className="px-0 py-0"
            />
          </div>
        `)}
      </div>
    <//>
  `;
}
