import { useT } from "../../../lib/i18n";
import { Panel, StatCard } from "@ironclaw/design-system";

function buildCards(t) {
  return [
    { key: "total", label: t("missions.summary.totalMissions"), tone: "muted" },
    { key: "active", label: t("missions.summary.active"), tone: "signal" },
    { key: "paused", label: t("missions.summary.paused"), tone: "warning" },
    { key: "threads", label: t("missions.summary.spawnedThreads"), tone: "success" },
  ];
}

export function MissionsSummaryStrip({ summary }) {
  const t = useT();
  const cards = buildCards(t);
  return (
    <Panel className="p-4 sm:p-5">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {cards.map((card) => (
          <div key={card.key} className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <StatCard
              label={card.label}
              value={summary[card.key] || 0}
              tone={card.tone}
              badgeLabel={card.key}
              detail={card.key === "total"
                ? t("missions.summary.completedFailed", { completed: summary.completed || 0, failed: summary.failed || 0 })
                : t("missions.summary.acrossProjects")}
              showDivider={false}
              className="px-0 py-0"
            />
          </div>
        ))}
      </div>
    </Panel>
  );
}
