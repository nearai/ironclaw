import { useT } from "../../../lib/i18n";
import { Panel, StatusPill } from "@ironclaw/design-system";
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
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.missions.label")}</div>
          <h2 className="mt-2 text-2xl font-medium tracking-tight text-[var(--v2-text-strong)]">{t("projects.missions.title")}</h2>
        </div>
        <div className="text-right text-xs uppercase tracking-[0.16em] text-[var(--v2-text-faint)]">
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
                    ? "border-[var(--v2-accent)]/35 bg-[var(--v2-accent-soft)]"
                    : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] hover:border-[var(--v2-accent)]/25 hover:bg-[var(--v2-surface-muted)]",
                ].join(" ")}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-lg font-medium text-[var(--v2-text-strong)]">{mission.name}</div>
                    <p className="mt-2 line-clamp-2 text-sm leading-6 text-[var(--v2-text-muted)]">{mission.goal}</p>
                  </div>
                  <StatusPill tone={missionTone(mission.status)} label={formatMissionStatus(mission.status, t)} />
                </div>
                <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
                  <span>{formatMissionCadence(mission, t)}</span>
                  <span>{t("projects.missions.threadCount", { count: mission.thread_count || 0 })}</span>
                  <span>{t("projects.missions.updated", { date: formatProjectDate(mission.updated_at, t) })}</span>
                </div>
              </button>
            ))
          : (
              <div className="rounded-[20px] border border-dashed border-[var(--v2-panel-border)] px-4 py-8 text-sm leading-6 text-[var(--v2-text-muted)]">
                {t("projects.missions.empty")}
              </div>
            )}
      </div>
    </Panel>
  );
}
