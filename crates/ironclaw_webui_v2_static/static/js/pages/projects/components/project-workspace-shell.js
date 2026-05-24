import { html } from "../../../lib/html.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";
import {
  compactCount,
  formatCurrency,
  formatProjectDate,
  formatProjectRelativeTime,
  healthTone,
} from "../lib/projects-presenters.js";
import { ProjectWidgets } from "./project-widgets.js";
import { ProjectMissionsColumn } from "./project-missions-column.js";
import { ProjectActivityColumn } from "./project-activity-column.js";
import { ProjectInspectorRail } from "./project-inspector-rail.js";

export function ProjectWorkspaceShell({
  project,
  overview,
  missions,
  threads,
  widgets,
  selectedMissionId,
  selectedThreadId,
  inspector,
  inspectorState,
  onSelectMission,
  onSelectThread,
  onClearInspector,
}) {
  return html`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]">
      <div className="space-y-5">
        <${Panel} className="overflow-hidden p-5 sm:p-6">
          <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
            <div className="min-w-0 max-w-3xl">
              <div className="flex flex-wrap items-center gap-3">
                <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-signal">Project workspace</div>
                <${StatusPill} tone=${healthTone(overview?.health)} label=${overview?.health || "steady"} />
              </div>
              <h2 className="mt-3 text-3xl font-semibold tracking-tight text-white">${project.name}</h2>
              <p className="mt-3 text-sm leading-6 text-iron-200">
                ${project.description || "This project is active, but it does not have a human-authored description yet."}
              </p>
            </div>

            <div className="grid gap-3 sm:grid-cols-2 xl:w-[320px] xl:grid-cols-1">
              <div className="rounded-2xl border border-white/10 bg-iron-950/60 px-4 py-3 text-sm text-iron-100">
                ${compactCount(overview?.active_missions || missions.length, "active mission")}
              </div>
              <div className="rounded-2xl border border-white/10 bg-iron-950/60 px-4 py-3 text-sm text-iron-100">
                ${compactCount(overview?.threads_today || 0, "thread")} today
              </div>
              <div className="rounded-2xl border border-white/10 bg-iron-950/60 px-4 py-3 text-sm text-iron-100">
                ${formatCurrency(overview?.cost_today_usd || 0)} spend today
              </div>
              <div className="rounded-2xl border border-white/10 bg-iron-950/60 px-4 py-3 text-sm text-iron-100">
                ${formatProjectRelativeTime(overview?.last_activity)}
              </div>
            </div>
          </div>

          <div className="mt-5 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            <div className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">Created</div>
              <div className="mt-2 text-sm leading-6 text-white">${formatProjectDate(project.created_at)}</div>
            </div>
            <div className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">Pending gates</div>
              <div className="mt-2 text-sm leading-6 text-white">${overview?.pending_gates || 0}</div>
            </div>
            <div className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">Failures 24h</div>
              <div className="mt-2 text-sm leading-6 text-white">${overview?.failures_24h || 0}</div>
            </div>
            <div className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
              <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">Total missions</div>
              <div className="mt-2 text-sm leading-6 text-white">${overview?.total_missions || missions.length}</div>
            </div>
          </div>
        <//>

        <${ProjectWidgets} widgets=${widgets} projectId=${project.id} />

        <div className="grid gap-5 2xl:grid-cols-2">
          <${ProjectMissionsColumn}
            missions=${missions}
            selectedMissionId=${selectedMissionId}
            onSelectMission=${onSelectMission}
          />
          <${ProjectActivityColumn}
            threads=${threads}
            selectedThreadId=${selectedThreadId}
            onSelectThread=${onSelectThread}
          />
        </div>
      </div>

      <${ProjectInspectorRail}
        project=${project}
        overview=${overview}
        missions=${missions}
        threads=${threads}
        inspector=${inspector}
        isLoading=${inspectorState.isLoading}
        error=${inspectorState.error}
        onClear=${onClearInspector}
        onOpenThread=${onSelectThread}
        onFireMission=${inspectorState.fireMission}
        onPauseMission=${inspectorState.pauseMission}
        onResumeMission=${inspectorState.resumeMission}
        isBusy=${inspectorState.isBusy}
      />
    </div>
  `;
}
