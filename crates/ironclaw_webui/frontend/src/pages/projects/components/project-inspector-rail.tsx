import { Button } from "@ironclaw/design-system";
import { Panel, StatusPill } from "@ironclaw/design-system";
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
      <Panel className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.snapshot.label")}</div>
            <h2 className="mt-2 text-2xl font-medium tracking-tight text-[var(--v2-text-strong)]">{project.name}</h2>
          </div>
          <StatusPill
            tone={healthTone(overview?.health)}
            label={formatProjectHealth(overview?.health || "steady", t)}
          />
        </div>
        <p className="mt-4 text-sm leading-6 text-[var(--v2-text)]">{project.description || t("projects.snapshot.noDescription")}</p>

        <div className="mt-5 grid gap-3 sm:grid-cols-2">
          <div className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/60 p-3 text-sm text-[var(--v2-text-strong)]">
            {t("projects.snapshot.activePausedMissions", { active: counts.active, paused: counts.paused })}
          </div>
          <div className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/60 p-3 text-sm text-[var(--v2-text-strong)]">
            {t("projects.snapshot.threadsGates", { threads: threads.length, gates: overview?.pending_gates || 0 })}
          </div>
        </div>
      </Panel>

      {project.goals?.length
        ? (
            <Panel className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.snapshot.goals")}</div>
              <div className="mt-4 space-y-2 text-sm leading-6 text-[var(--v2-text)]">
                {project.goals.map((goal, index) => (<div key={index} className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/60 px-3 py-2">{goal}</div>))}
              </div>
            </Panel>
          )
        : null}

      {project.metrics?.length
        ? (
            <Panel className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.snapshot.metrics")}</div>
              <div className="mt-4 space-y-3">
                {project.metrics.map((metric, index) => (
                  <div key={index} className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/60 p-3">
                    <div className="text-sm font-medium text-[var(--v2-text-strong)]">{metric.name}</div>
                    <div className="mt-2 text-sm text-[var(--v2-text)]">{formatMetricValue(metric, t)}</div>
                    {metric.updated_at && (
                      <div className="mt-2 font-mono text-[10px] uppercase tracking-[0.16em] text-[var(--v2-text-faint)]">
                        {t("projects.snapshot.updated", { date: formatProjectDate(metric.updated_at, t) })}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </Panel>
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
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.inspector.label")}</div>
        {inspector?.type && (<Button variant="ghost" size="sm" onClick={onClear}>{t("projects.inspector.clearFocus")}</Button>)}
      </div>

      {isLoading
        ? (<div className="space-y-4">{[1, 2].map((index) => (<div key={index} className="v2-skeleton h-48 rounded-[20px]" />))}</div>)
        : error
          ? (<div className="rounded-xl border border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-4 py-3 text-sm text-[var(--v2-danger-text)]">{error.message}</div>)
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
