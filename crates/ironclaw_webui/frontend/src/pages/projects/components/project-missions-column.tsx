import { useT } from "../../../lib/i18n";
import { Panel, StatusPill } from "../../../design-system/primitives";
import {
  formatMissionCadence,
  formatMissionStatus,
  formatProjectDate,
  missionTone,
  missionStatusCounts,
} from "../lib/projects-presenters";

export function ProjectMissionsColumn({ missions, selectedMissionId, onSelectMission }) {
  const t = useT();
  const counts = missionStatusCounts(missions);

  return (
    <Panel className="p-4 sm:p-5">
      <div className="flex items-end justify-between gap-4">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.missions.label")}</div>
          <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">{t("projects.missions.title")}</h2>
        </div>
        <div className="text-right text-xs uppercase tracking-[0.16em] text-iron-400">
          <div>{t("projects.missions.activePaused", { active: counts.active, paused: counts.paused })}</div>
          <div className="mt-1">{t("projects.missions.completedFailed", { completed: counts.completed, failed: counts.failed })}</div>
        </div>
      </div>

      <div className="mt-5 space-y-3">
        {missions.length
          ? missions.map((mission) => (
              <button
                key={mission.id}
                onClick={() => onSelectMission(mission.id)}
                className={[
                  "w-full rounded-[20px] border p-4 text-left",
                  selectedMissionId === mission.id
                    ? "border-signal/35 bg-signal/10"
                    : "border-white/10 bg-white/[0.025] hover:border-signal/25 hover:bg-white/[0.045]",
                ].join(" ")}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-lg font-semibold text-white">{mission.name}</div>
                    <p className="mt-2 line-clamp-2 text-sm leading-6 text-iron-300">{mission.goal}</p>
                  </div>
                  <StatusPill tone={missionTone(mission.status)} label={formatMissionStatus(mission.status, t)} />
                </div>
                <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                  <span>{formatMissionCadence(mission, t)}</span>
                  <span>{t("projects.missions.threadCount", { count: mission.thread_count || 0 })}</span>
                  <span>{t("projects.missions.updated", { date: formatProjectDate(mission.updated_at, t) })}</span>
                </div>
              </button>
            ))
          : (
              <div className="rounded-[20px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                {t("projects.missions.empty")}
              </div>
            )}
      </div>
    </Panel>
  );
}
