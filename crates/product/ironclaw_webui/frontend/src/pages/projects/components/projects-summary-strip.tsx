import { useT } from "../../../lib/i18n";
import { Panel, StatusPill } from "../../../design-system/primitives";
import { formatProjectState, summarizeOverview } from "../lib/projects-presenters";

const metricTone = {
  projects: "muted",
  active: "success",
  archived: "muted",
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
    },
    {
      key: "active",
      label: formatProjectState("active", t),
      badgeLabel: t("projects.summary.projectsBadge"),
      value: summary.activeProjects,
    },
    {
      key: "archived",
      label: formatProjectState("archived", t),
      badgeLabel: t("projects.summary.projectsBadge"),
      value: summary.archivedProjects,
    },
  ];

  return (
    <Panel data-testid="projects-summary" className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-3">
        {cards.map((card) => (
          <div
            key={card.key}
            data-summary-kind={card.key}
            className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{card.label}</div>
              <StatusPill tone={metricTone[card.key]} label={card.badgeLabel} />
            </div>
            <div
              data-testid="projects-summary-value"
              className="mt-4 text-display font-semibold tracking-tight text-[var(--v2-text-strong)]"
            >
              {card.value}
            </div>
          </div>
        ))}
      </div>
    </Panel>
  );
}
