import { Button } from "../../../design-system/button";
import { Panel, StatusPill } from "../../../design-system/primitives";
import { useT } from "../../../lib/i18n";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer";
import {
  formatMissionCadence,
  formatMissionStatus,
  formatProjectDate,
  missionTone,
} from "../lib/projects-presenters";

function MetaCard({ label, value }) {
  return (
    <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">{label}</div>
      <div className="mt-2 text-sm leading-6 text-white">{value}</div>
    </div>
  );
}

export function ProjectMissionInspector({
  mission,
  onFire,
  onPause,
  onResume,
  onOpenThread,
  isBusy,
}) {
  const t = useT();
  const actionButtons = [];
  if (mission.status === "Active") {
    actionButtons.push((<Button key="fire" onClick={() => onFire(mission.id)} disabled={isBusy}>{t("projects.mission.fireNow")}</Button>));
    actionButtons.push((<Button key="pause" variant="secondary" onClick={() => onPause(mission.id)} disabled={isBusy}>{t("projects.mission.pause")}</Button>));
  } else if (mission.status === "Paused") {
    actionButtons.push((<Button key="resume" onClick={() => onResume(mission.id)} disabled={isBusy}>{t("projects.mission.resume")}</Button>));
    actionButtons.push((<Button key="fire" variant="secondary" onClick={() => onFire(mission.id)} disabled={isBusy}>{t("projects.mission.runOnce")}</Button>));
  } else {
    actionButtons.push((<Button key="retry" onClick={() => onFire(mission.id)} disabled={isBusy}>{t("projects.mission.runAgain")}</Button>));
  }

  return (
    <div className="space-y-4">
      <Panel className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.mission.dossier")}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">{mission.name}</h2>
          </div>
          <StatusPill tone={missionTone(mission.status)} label={formatMissionStatus(mission.status, t)} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <MetaCard label={t("projects.mission.cadence")} value={formatMissionCadence(mission, t)} />
          <MetaCard label={t("projects.mission.threadsToday")} value={t("projects.mission.threadsTodayValue", { count: mission.threads_today || 0, max: mission.max_threads_per_day || "∞" })} />
          <MetaCard label={t("projects.mission.nextFire")} value={mission.next_fire_at ? formatProjectDate(mission.next_fire_at, t) : t("projects.mission.notScheduled")} />
          <MetaCard label={t("projects.mission.created")} value={formatProjectDate(mission.created_at, t)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">{actionButtons}</div>
      </Panel>

      <Panel className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.mission.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <MarkdownRenderer content={mission.goal || t("projects.mission.noGoal")} />
        </div>
      </Panel>

      {mission.current_focus
        ? (
            <Panel className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.mission.currentFocus")}</div>
              <div className="mt-4 text-sm leading-6 text-iron-200">
                <MarkdownRenderer content={mission.current_focus} />
              </div>
            </Panel>
          )
        : null}

      {mission.success_criteria
        ? (
            <Panel className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.mission.successCriteria")}</div>
              <div className="mt-4 text-sm leading-6 text-iron-200">
                <MarkdownRenderer content={mission.success_criteria} />
              </div>
            </Panel>
          )
        : null}

      {mission.approach_history?.length
        ? (
            <Panel className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.mission.approachHistory")}</div>
              <div className="mt-4 space-y-3">
                {mission.approach_history.map((entry, index) => (
                  <div key={index} className="rounded-2xl border border-white/8 bg-iron-950/60 p-4">
                    <div className="mb-3 text-xs uppercase tracking-[0.16em] text-iron-400">{t("projects.mission.runLabel", { number: index + 1 })}</div>
                    <MarkdownRenderer content={entry} />
                  </div>
                ))}
              </div>
            </Panel>
          )
        : null}

      {mission.threads?.length
        ? (
            <Panel className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.mission.spawnedThreads")}</div>
              <div className="mt-4 space-y-3">
                {mission.threads.map((thread) => {
                  const status = thread.state === "Running" ? "Active" : thread.state === "Failed" ? "Failed" : "Completed";
                  return (
                    <button
                      key={thread.id}
                      onClick={() => onOpenThread(thread.id)}
                      className="w-full rounded-2xl border border-white/8 bg-iron-950/60 p-4 text-left hover:border-signal/30 hover:bg-white/[0.05]"
                    >
                      <div className="flex items-center justify-between gap-3">
                        <div className="min-w-0 truncate text-sm font-semibold text-white">{thread.goal}</div>
                        <StatusPill tone={missionTone(status)} label={formatMissionStatus(status, t)} />
                      </div>
                    </button>
                  );
                })}
              </div>
            </Panel>
          )
        : null}
    </div>
  );
}
