import { useT } from "../../../lib/i18n";
import { Button } from "../../../design-system/button";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer";
import { formatMissionDate, missionTone } from "../lib/missions-presenters";

function MetaCard({ label, value }) {
  return (
    <div className="rounded-xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">{label}</div>
      <div className="mt-2 text-sm leading-6 text-white">{value}</div>
    </div>
  );
}

function ActionButtons({ mission, isBusy, onFire, onPause, onResume }) {
  const t = useT();
  if (mission.status === "Active") {
    return (
      <>
      <Button onClick={() => onFire(mission.id)} disabled={isBusy}>{t("missions.action.fireNow")}</Button>
      <Button variant="secondary" onClick={() => onPause(mission.id)} disabled={isBusy}>{t("missions.action.pause")}</Button>
      </>
    );
  }

  if (mission.status === "Paused") {
    return (
      <>
      <Button onClick={() => onResume(mission.id)} disabled={isBusy}>{t("missions.action.resume")}</Button>
      <Button variant="secondary" onClick={() => onFire(mission.id)} disabled={isBusy}>{t("missions.action.runOnce")}</Button>
      </>
    );
  }

  return (<Button onClick={() => onFire(mission.id)} disabled={isBusy}>{t("missions.action.runAgain")}</Button>);
}

export function MissionDetailPanel({
  mission,
  isLoading,
  error,
  isBusy,
  onFire,
  onPause,
  onResume,
  onOpenProject,
  onOpenThread,
}) {
  const t = useT();
  if (isLoading) {
    return (
      <div className="space-y-4">
        {[1, 2, 3].map((index) => (<div key={index} className="v2-skeleton h-36 rounded-xl" />))}
      </div>
    );
  }

  if (error || !mission) {
    return (
      <EmptyPanel
        title={t("missions.unavailable")}
        description={error?.message || t("missions.unavailableDesc")}
      />
    );
  }

  return (
    <div className="space-y-4">
      <Panel className="p-4 sm:p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.dossier")}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">{mission.name}</h2>
            {mission.project && (
              <button
                type="button"
                onClick={() => onOpenProject(mission.project.id)}
                className="mt-2 text-sm text-signal underline-offset-4 hover:underline"
              >
                {mission.project.name}
              </button>
            )}
          </div>
          <StatusPill tone={missionTone(mission.status)} label={mission.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <MetaCard label={t("missions.meta.cadence")} value={mission.cadence_description || mission.cadence_type || t("missions.meta.manual")} />
          <MetaCard label={t("missions.meta.threadsToday")} value={`${mission.threads_today || 0} / ${mission.max_threads_per_day || t("missions.meta.unlimited")}`} />
          <MetaCard label={t("missions.meta.nextFire")} value={formatMissionDate(mission.next_fire_at)} />
          <MetaCard label={t("missions.meta.updated")} value={formatMissionDate(mission.updated_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">
          <ActionButtons
            mission={mission}
            isBusy={isBusy}
            onFire={onFire}
            onPause={onPause}
            onResume={onResume}
          />
        </div>
      </Panel>

      <Panel className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <MarkdownRenderer content={mission.goal || t("missions.noGoal")} />
        </div>
      </Panel>

      {mission.current_focus && (
        <Panel className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <MarkdownRenderer content={mission.current_focus} />
          </div>
        </Panel>
      )}

      {mission.success_criteria && (
        <Panel className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <MarkdownRenderer content={mission.success_criteria} />
          </div>
        </Panel>
      )}

      {mission.threads?.length ? (
        <Panel className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.spawnedThreads")}</div>
          <div className="mt-4 space-y-3">
            {mission.threads.map((thread) => (
              <button
                key={thread.id}
                type="button"
                onClick={() => onOpenThread(thread)}
                className="w-full rounded-xl border border-white/8 bg-iron-950/60 p-4 text-left hover:border-signal/30 hover:bg-white/[0.05]"
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0 truncate text-sm font-semibold text-white">{thread.title || thread.goal}</div>
                  <StatusPill tone={missionTone(thread.state === "Running" ? "Active" : thread.state === "Failed" ? "Failed" : "Completed")} label={thread.state} />
                </div>
              </button>
            ))}
          </div>
        </Panel>
      ) : null}
    </div>
  );
}
