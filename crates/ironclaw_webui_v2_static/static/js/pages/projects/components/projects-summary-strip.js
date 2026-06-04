import { html } from "../../../lib/html.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";
import { formatCurrency, summarizeOverview } from "../lib/projects-presenters.js";

const metricTone = {
  projects: "muted",
  missions: "signal",
  attention: "warning",
  spend: "success",
};

export function ProjectsSummaryStrip({ overview }) {
  const summary = summarizeOverview(overview);
  const cards = [
    {
      key: "projects",
      label: "Projects",
      value: summary.totalProjects,
      detail: `${summary.threadsToday} threads active today`,
    },
    {
      key: "missions",
      label: "Active missions",
      value: summary.activeMissions,
      detail: `${summary.pendingGates} gates across the workspace`,
    },
    {
      key: "attention",
      label: "Attention queue",
      value: summary.attentionCount,
      detail: `${summary.failures24h} failures in the last 24h`,
    },
    {
      key: "spend",
      label: "Spend today",
      value: formatCurrency(summary.totalSpend),
      detail: `${summary.totalProjects ? "Across every project" : "Waiting for activity"}`,
    },
  ];

  return html`
    <${Panel} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        ${cards.map((card) => html`
          <div key=${card.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${card.label}</div>
              <${StatusPill} tone=${metricTone[card.key]} label=${card.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">${card.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">${card.detail}</p>
          </div>
        `)}
      </div>
    <//>
  `;
}
