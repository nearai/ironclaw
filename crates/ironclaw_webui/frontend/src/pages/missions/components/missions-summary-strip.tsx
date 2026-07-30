import { useT } from "../../../lib/i18n";
import { StatStrip, StatTile, type BadgeTone } from "@ironclaw/ui";

function buildCards(t): { key: string; label: string; tone: BadgeTone }[] {
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
    <StatStrip columns={4}>
      {cards.map((card) => (
        <StatTile
          key={card.key}
          label={card.label}
          value={summary[card.key] || 0}
          tone={card.tone}
          badgeLabel={card.key}
          detail={
            card.key === "total"
              ? t("missions.summary.completedFailed", { completed: summary.completed || 0, failed: summary.failed || 0 })
              : t("missions.summary.acrossProjects")
          }
        />
      ))}
    </StatStrip>
  );
}
