import { useT } from "../../../lib/i18n";
import { Panel, StatCard } from "@ironclaw/design-system";
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
          <div key={card.key} className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <StatCard
              label={card.label}
              value={card.value}
              tone={metricTone[card.key]}
              badgeLabel={card.badgeLabel}
              detail={card.detail}
              showDivider={false}
              className="px-0 py-0"
            />
          </div>
        ))}
      </div>
    </Panel>
  );
}
