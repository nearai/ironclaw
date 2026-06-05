import { html } from "../../../lib/html.js";
import { Button } from "../../../design-system/button.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";
import {
  compactCount,
  formatMetricValue,
  formatProjectDate,
  healthTone,
  missionStatusCounts,
} from "../lib/projects-presenters.js";
import { ProjectMissionInspector } from "./project-mission-inspector.js";
import { ProjectThreadInspector } from "./project-thread-inspector.js";

function ProjectSnapshot({ project, missions, threads, overview }) {
  const counts = missionStatusCounts(missions);

  return html`
    <div className="space-y-4">
      <${Panel} className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Project snapshot</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">${project.name}</h2>
          </div>
          <${StatusPill} tone=${healthTone(overview?.health)} label=${overview?.health || "steady"} />
        </div>
        <p className="mt-4 text-sm leading-6 text-iron-200">${project.description || "No project description yet."}</p>

        <div className="mt-5 grid gap-3 sm:grid-cols-2">
          <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3 text-sm text-iron-100">
            ${compactCount(counts.active, "active mission")} / ${compactCount(counts.paused, "paused mission")}
          </div>
          <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3 text-sm text-iron-100">
            ${compactCount(threads.length, "thread")} / ${compactCount(overview?.pending_gates || 0, "gate")}
          </div>
        </div>
      <//>

      ${project.goals?.length
        ? html`
            <${Panel} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Goals</div>
              <div className="mt-4 space-y-2 text-sm leading-6 text-iron-200">
                ${project.goals.map((goal, index) => html`<div key=${index} className="rounded-2xl border border-white/8 bg-iron-950/60 px-3 py-2">${goal}</div>`)}
              </div>
            <//>
          `
        : null}

      ${project.metrics?.length
        ? html`
            <${Panel} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Metrics</div>
              <div className="mt-4 space-y-3">
                ${project.metrics.map((metric, index) => html`
                  <div key=${index} className="rounded-2xl border border-white/8 bg-iron-950/60 p-3">
                    <div className="text-sm font-semibold text-white">${metric.name}</div>
                    <div className="mt-2 text-sm text-iron-200">${formatMetricValue(metric)}</div>
                    ${metric.updated_at && html`
                      <div className="mt-2 font-mono text-[10px] uppercase tracking-[0.16em] text-iron-400">
                        Updated ${formatProjectDate(metric.updated_at)}
                      </div>
                    `}
                  </div>
                `)}
              </div>
            <//>
          `
        : null}
    </div>
  `;
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
  return html`
    <aside className="space-y-4">
      <div className="flex items-center justify-between gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Inspector</div>
        ${inspector?.type && html`<${Button} variant="ghost" className="h-8 px-3 text-xs" onClick=${onClear}>Clear focus<//>`}
      </div>

      ${isLoading
        ? html`<div className="space-y-4">${[1, 2].map((index) => html`<div key=${index} className="v2-skeleton h-48 rounded-[20px]" />`)}</div>`
        : error
          ? html`<div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">${error.message}</div>`
          : inspector?.type === "mission"
            ? html`
                <${ProjectMissionInspector}
                  mission=${inspector.mission}
                  onFire=${onFireMission}
                  onPause=${onPauseMission}
                  onResume=${onResumeMission}
                  onOpenThread=${onOpenThread}
                  isBusy=${isBusy}
                />
              `
            : inspector?.type === "thread"
              ? html`<${ProjectThreadInspector} thread=${inspector.thread} />`
              : html`<${ProjectSnapshot} project=${project} missions=${missions} threads=${threads} overview=${overview} />`}
    </aside>
  `;
}
