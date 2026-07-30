import { Button, Callout, Card, SectionHeader, SkeletonList, Badge } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";
import {
  formatProjectHealth,
  formatMetricValue,
  formatProjectDate,
  healthTone,
  missionStatusCounts,
} from "../lib/projects-presenters";
import { ProjectMissionInspector } from "./project-mission-inspector";
import { ProjectThreadInspector } from "./project-thread-inspector";

function ProjectSnapshot({ project, missions, threads, overview, t }) {
  const counts = missionStatusCounts(missions);

  return (
    <div className="space-y-4">
      <Card className="p-4 sm:p-5">
        <SectionHeader
          eyebrow={t("projects.snapshot.label")}
          title={project.name}
          actions={
            <Badge
              tone={healthTone(overview?.health)}
              label={formatProjectHealth(overview?.health || "steady", t)}
            />
          }
        />
        <p className="mt-4 text-sm leading-6 text-iron-200">{project.description || t("projects.snapshot.noDescription")}</p>

        <div className="mt-5 grid gap-3 sm:grid-cols-2">
          <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3 text-sm text-iron-100">
            {t("projects.snapshot.activePausedMissions", { active: counts.active, paused: counts.paused })}
          </div>
          <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3 text-sm text-iron-100">
            {t("projects.snapshot.threadsGates", { threads: threads.length, gates: overview?.pending_gates || 0 })}
          </div>
        </div>
      </Card>

      {project.goals?.length
        ? (
            <Card className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.snapshot.goals")}</div>
              <div className="mt-4 space-y-2 text-sm leading-6 text-iron-200">
                {project.goals.map((goal, index) => (<div key={index} className="rounded-2xl border border-white/8 bg-iron-950/60 px-3 py-2">{goal}</div>))}
              </div>
            </Card>
          )
        : null}

      {project.metrics?.length
        ? (
            <Card className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.snapshot.metrics")}</div>
              <div className="mt-4 space-y-3">
                {project.metrics.map((metric, index) => (
                  <div key={index} className="rounded-2xl border border-white/8 bg-iron-950/60 p-3">
                    <div className="text-sm font-semibold text-white">{metric.name}</div>
                    <div className="mt-2 text-sm text-iron-200">{formatMetricValue(metric, t)}</div>
                    {metric.updated_at && (
                      <div className="mt-2 font-mono text-[10px] uppercase tracking-[0.16em] text-iron-400">
                        {t("projects.snapshot.updated", { date: formatProjectDate(metric.updated_at, t) })}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </Card>
          )
        : null}
    </div>
  );
}

export function ProjectInspectorRail({
  project,
  overview,
  missions,
  threads,
  inspector,
  isLoading,
  error,
  onClear,
  onOpenThread,
  onFireMission,
  onPauseMission,
  onResumeMission,
  isBusy,
}) {
  const t = useT();

  return (
    <aside className="space-y-4">
      <div className="flex items-center justify-between gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.inspector.label")}</div>
        {inspector?.type && (<Button variant="ghost" className="h-8 px-3 text-xs" onClick={onClear}>{t("projects.inspector.clearFocus")}</Button>)}
      </div>

      {isLoading
        ? (<SkeletonList label={t("projects.inspector.loading")} count={2} itemClassName="h-48 rounded-[20px]" />)
        : error
          ? (<Callout tone="danger">{error.message}</Callout>)
          : inspector?.type === "mission"
            ? (
                <ProjectMissionInspector
                  mission={inspector.mission}
                  onFire={onFireMission}
                  onPause={onPauseMission}
                  onResume={onResumeMission}
                  onOpenThread={onOpenThread}
                  isBusy={isBusy}
                />
              )
            : inspector?.type === "thread"
              ? (<ProjectThreadInspector thread={inspector.thread} />)
              : (<ProjectSnapshot project={project} missions={missions} threads={threads} overview={overview} t={t} />)}
    </aside>
  );
}
