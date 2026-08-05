import { useT } from "../../../lib/i18n";
import { Panel, StatusPill } from "../../../design-system/primitives";

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
          <div key={card.key} className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{card.label}</div>
              <StatusPill tone={card.tone} label={card.key} />
            </div>
            <div className="mt-4 text-3xl font-semibold tracking-tight text-white">{summary[card.key] || 0}</div>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              {card.key === "total"
                ? t("missions.summary.completedFailed", { completed: summary.completed || 0, failed: summary.failed || 0 })
                : t("missions.summary.acrossProjects")}
            </p>
          </div>
        ))}
      </div>
    </Panel>
  );
}
