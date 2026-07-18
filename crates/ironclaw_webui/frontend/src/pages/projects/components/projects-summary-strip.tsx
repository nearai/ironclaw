import { useT } from "../../../lib/i18n";
import { Panel, StatusPill } from "../../../design-system/primitives";
import { formatCurrency, summarizeOverview } from "../lib/projects-presenters";

const metricTone = {
  projects: "muted",
  attention: "warning",
  spend: "success",
};

export function ProjectsSummaryStrip({ overview }) {
  const t = useT();
  const summary = summarizeOverview(overview);
  const cards = [
    {
      key: "projects",
      label: t("projects.summary.projects"),
      badgeLabel: t("projects.summary.projectsBadge"),
      value: summary.totalProjects,
      detail: t("projects.summary.threadsActiveToday", { count: summary.threadsToday }),
    },
    {
      key: "attention",
      label: t("projects.summary.attentionQueue"),
      badgeLabel: t("projects.summary.attentionBadge"),
      value: summary.attentionCount,
      detail: t("projects.summary.failures24h", { count: summary.failures24h }),
    },
    {
      key: "spend",
      label: t("projects.summary.spendToday"),
      badgeLabel: t("projects.summary.spendBadge"),
      value: formatCurrency(summary.totalSpend),
      detail: summary.totalProjects
        ? t("projects.summary.acrossEveryProject")
        : t("projects.summary.waitingForActivity"),
    },
  ];

  return (
    <Panel className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {cards.map((card) => (
          <div key={card.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{card.label}</div>
              <StatusPill tone={metricTone[card.key]} label={card.badgeLabel} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">{card.value}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">{card.detail}</p>
          </div>
        ))}
      </div>
    </Panel>
  );
}
