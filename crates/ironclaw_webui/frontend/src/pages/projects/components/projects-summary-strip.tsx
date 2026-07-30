import { useT } from "../../../lib/i18n";
import { StatStrip, StatTile } from "@ironclaw/ui";
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
    <StatStrip columns={3}>
      {cards.map((card) => (
        <StatTile
          key={card.key}
          label={card.label}
          value={card.value}
          tone={metricTone[card.key]}
          badgeLabel={card.badgeLabel}
          detail={card.detail}
        />
      ))}
    </StatStrip>
  );
}
