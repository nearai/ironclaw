import { html } from "../../../lib/html.js";
import { Panel, StatCard } from "../../../design-system/primitives.js";

export function AutomationsSummaryStrip({ summary }) {
  const cards = [
    {
      key: "scheduled",
      label: "Scheduled",
      value: summary?.scheduled ?? 0,
      tone: "muted",
      detail: "Scheduled automations visible to this agent.",
    },
    {
      key: "active",
      label: "Active",
      value: summary?.active ?? 0,
      tone: "signal",
      detail: "Enabled schedules waiting for their next run.",
    },
    {
      key: "paused",
      label: "Paused",
      value: summary?.paused ?? 0,
      tone: "warning",
      detail: "Schedules currently not expected to run.",
    },
    {
      key: "nextRun",
      label: "Next run",
      value: summary?.nextRun || "None",
      tone: "info",
      detail: "Soonest scheduled run in this list.",
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
