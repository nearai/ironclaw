import { useT } from "../../../lib/i18n";
import {
  Button,
  DetailList,
  DetailRow,
  EmptyPanel,
  Card,
  SkeletonList,
  Badge,
} from "@ironclaw/ui";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer";
import { formatMissionDate, missionTone } from "../lib/missions-presenters";

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
      <SkeletonList
        label={t("missions.detail.loading")}
        itemClassName="h-36 rounded-xl"
      />
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
      <Card className="p-4 sm:p-5">
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
          <Badge tone={missionTone(mission.status)} label={mission.status} />
        </div>

        <DetailList className="mt-2 grid gap-x-6 sm:grid-cols-2">
          <DetailRow layout="stacked" term={t("missions.meta.cadence")}>
            {mission.cadence_description || mission.cadence_type || t("missions.meta.manual")}
          </DetailRow>
          <DetailRow layout="stacked" term={t("missions.meta.threadsToday")}>
            {`${mission.threads_today || 0} / ${mission.max_threads_per_day || t("missions.meta.unlimited")}`}
          </DetailRow>
          <DetailRow layout="stacked" term={t("missions.meta.nextFire")}>
            {formatMissionDate(mission.next_fire_at)}
          </DetailRow>
          <DetailRow layout="stacked" term={t("missions.meta.updated")}>
            {formatMissionDate(mission.updated_at)}
          </DetailRow>
        </DetailList>

        <div className="mt-5 flex flex-wrap gap-2">
          <ActionButtons
            mission={mission}
            isBusy={isBusy}
            onFire={onFire}
            onPause={onPause}
            onResume={onResume}
          />
        </div>
      </Card>

      <Card className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.brief")}</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <MarkdownRenderer content={mission.goal || t("missions.noGoal")} />
        </div>
      </Card>

      {mission.current_focus && (
        <Card className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.currentFocus")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <MarkdownRenderer content={mission.current_focus} />
          </div>
        </Card>
      )}

      {mission.success_criteria && (
        <Card className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.successCriteria")}</div>
          <div className="mt-4 text-sm leading-6 text-iron-200">
            <MarkdownRenderer content={mission.success_criteria} />
          </div>
        </Card>
      )}

      {mission.threads?.length ? (
        <Card className="p-4 sm:p-5">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("missions.spawnedThreads")}</div>
          <div className="mt-4 space-y-3">
            {mission.threads.map((thread) => (
              <button
                key={thread.id}
                type="button"
                onClick={() => onOpenThread(thread)}
                className="w-full rounded-xl border border-white/8 bg-iron-950/60 p-4 text-left transition-colors hover:border-signal/30 hover:bg-white/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0 truncate text-sm font-semibold text-white">{thread.title || thread.goal}</div>
                  <Badge tone={missionTone(thread.state === "Running" ? "Active" : thread.state === "Failed" ? "Failed" : "Completed")} label={thread.state} />
                </div>
              </button>
            ))}
          </div>
        </Card>
      ) : null}
    </div>
  );
}
