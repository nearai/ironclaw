import { html } from "../../../lib/html.js";
import { Panel, StatCard } from "../../../design-system/primitives.js";

const SUMMARY_CARDS = [
  { key: "total", label: "Total jobs", tone: "muted", detail: "All tracked work across agent and sandbox execution." },
  { key: "pending", label: "Pending", tone: "warning", detail: "Queued work waiting for a worker or container slot." },
  { key: "in_progress", label: "In progress", tone: "signal", detail: "Actively running jobs and live bridges." },
  { key: "completed", label: "Completed", tone: "success", detail: "Finished without intervention." },
  { key: "failed", label: "Failed", tone: "danger", detail: "Runs that terminated with an error or interruption." },
  { key: "stuck", label: "Stuck", tone: "danger", detail: "Agent work needing recovery or operator attention." },
];

export function JobsSummaryStrip({ summary }) {
  return html`
    <${Panel} className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6">
        ${SUMMARY_CARDS.map((card) => html`
          <div
            key=${card.key}
            className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
          >
            <${StatCard}
              label=${card.label}
              value=${summary?.[card.key] ?? 0}
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
