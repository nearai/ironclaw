import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import { formatMissionDate, missionTone } from "../lib/missions-presenters.js";

function buildStatusOptions(t) {
  return [
    { value: "all", label: t("missions.filter.allStatuses") },
    { value: "Active", label: t("missions.status.active") },
    { value: "Paused", label: t("missions.status.paused") },
    { value: "Failed", label: t("missions.status.failed") },
    { value: "Completed", label: t("missions.status.completed") },
  ];
}

function FilterSelect({ value, onChange, children, label }) {
  return html`
    <label className="min-w-[160px] flex-1 sm:flex-none">
      <span className="sr-only">${label}</span>
      <select
        value=${value}
        onChange=${(event) => onChange(event.target.value)}
        className="v2-select h-11 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none focus:border-signal/40"
      >
        ${children}
      </select>
    </label>
  `;
}

function MissionRow({ mission, selectedMissionId, onSelectMission, onOpenProject }) {
  const t = useT();
  const selected = selectedMissionId === mission.id;

  return html`
    <div
      className=${[
        "w-full rounded-xl border p-4 text-left",
        selected
          ? "border-signal/35 bg-signal/10"
          : "border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80",
      ].join(" ")}
    >
      <button type="button" onClick=${() => onSelectMission(mission.id)} className="block w-full text-left">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">${mission.name}</div>
              <${StatusPill} tone=${missionTone(mission.status)} label=${mission.status} />
            </div>
            <p className="mt-2 line-clamp-2 text-sm leading-6 text-iron-300">${mission.goal || t("missions.noGoal")}</p>
          </div>
          <div className="shrink-0 text-right font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
            <div>${mission.cadence_description || mission.cadence_type || "manual"}</div>
            <div className="mt-1">${t("missions.threadCount", { count: mission.thread_count || 0 })}</div>
          </div>
        </div>
      </button>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-iron-700 pt-3">
        <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
          ${t("missions.updated", { value: formatMissionDate(mission.updated_at) })}
        </span>
        <${Button}
          variant="ghost"
          onClick=${(event) => {
            event.stopPropagation();
            onOpenProject(mission.project.id);
          }}
        >
          ${mission.project.name}
        <//>
      </div>
    </div>
  `;
}

export function MissionsList({
  missions,
  totalMissions,
  selectedMissionId,
  search,
  onSearchChange,
  statusFilter,
  onStatusFilterChange,
  projectFilter,
  onProjectFilterChange,
  projectOptions,
  onSelectMission,
  onOpenProject,
}) {
  const t = useT();
  const statusOptions = buildStatusOptions(t);
  return html`
    <${Panel} className="p-4 sm:p-5">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${t("missions.title")}</div>
          <h1 className="mt-2 text-3xl font-semibold tracking-tight text-iron-100">${t("missions.subtitle")}</h1>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
            ${t("missions.summary", { missions: totalMissions, projects: projectOptions.length })}
          </p>
        </div>
      </div>

      <div className="mt-5 flex flex-wrap gap-3">
        <input
          value=${search}
          onChange=${(event) => onSearchChange(event.target.value)}
          placeholder=${t("missions.searchPlaceholder")}
          className="h-11 min-w-[220px] flex-1 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/40"
        />
        <${FilterSelect} value=${statusFilter} onChange=${onStatusFilterChange} label=${t("missions.filter.status")}>
          ${statusOptions.map((status) => html`<option key=${status.value} value=${status.value}>${status.label}<//>`)}
        <//>
        <${FilterSelect} value=${projectFilter} onChange=${onProjectFilterChange} label=${t("missions.filter.project")}>
          <option value="all">${t("missions.filter.allProjects")}</option>
          ${projectOptions.map((project) => html`<option key=${project.id} value=${project.id}>${project.name}<//>`)}
        <//>
      </div>

      <div className="mt-5 space-y-3">
        ${missions.length
          ? missions.map((mission) => html`
              <${MissionRow}
                key=${mission.id}
                mission=${mission}
                selectedMissionId=${selectedMissionId}
                onSelectMission=${onSelectMission}
                onOpenProject=${onOpenProject}
              />
            `)
          : html`
              <${EmptyPanel}
                title=${t("missions.emptyTitle")}
                description=${t("missions.emptyDesc")}
                boxed=${false}
              />
            `}
      </div>
    <//>
  `;
}
