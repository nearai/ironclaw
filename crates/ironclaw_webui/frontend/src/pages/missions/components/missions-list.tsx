import { useT } from "../../../lib/i18n";
import {
  Button,
  EmptyPanel,
  Card,
  SearchInput,
  SectionHeader,
  Select,
  Badge,
  Toolbar,
} from "@ironclaw/ui";
import { formatMissionDate, missionTone } from "../lib/missions-presenters";

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
  return (
    <label className="min-w-[160px] flex-1 sm:flex-none">
      <span className="sr-only">{label}</span>
      <Select
        size="sm"
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
      >
        {children}
      </Select>
    </label>
  );
}

function MissionRow({ mission, selectedMissionId, onSelectMission, onOpenProject }) {
  const t = useT();
  const selected = selectedMissionId === mission.id;

  return (
    <div
      className={[
        "w-full rounded-xl border p-4 text-left",
        selected
          ? "border-signal/35 bg-signal/10"
          : "border-iron-700 bg-iron-800/50 hover:border-signal/25 hover:bg-iron-800/80",
      ].join(" ")}
    >
      <button type="button" onClick={() => onSelectMission(mission.id)} className="block w-full rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 truncate text-lg font-semibold text-iron-100">{mission.name}</div>
              <Badge tone={missionTone(mission.status)} label={mission.status} />
            </div>
            <p className="mt-2 line-clamp-2 text-sm leading-6 text-iron-300">{mission.goal || t("missions.noGoal")}</p>
          </div>
          <div className="shrink-0 text-right font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
            <div>{mission.cadence_description || mission.cadence_type || "manual"}</div>
            <div className="mt-1">{t("missions.threadCount", { count: mission.thread_count || 0 })}</div>
          </div>
        </div>
      </button>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-iron-700 pt-3">
        <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
          {t("missions.updated", { value: formatMissionDate(mission.updated_at) })}
        </span>
        <Button
          variant="ghost"
          onClick={(event) => {
            event.stopPropagation();
            onOpenProject(mission.project.id);
          }}
        >
          {mission.project.name}
        </Button>
      </div>
    </div>
  );
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
  return (
    <Card className="p-4 sm:p-5">
      <SectionHeader
        eyebrow={t("missions.title")}
        title={t("missions.subtitle")}
        titleAs="h1"
        description={t("missions.summary", { missions: totalMissions, projects: projectOptions.length })}
      />

      <Toolbar className="mt-5">
        <SearchInput
          label={t("missions.searchPlaceholder")}
          value={search}
          onChange={(event) => onSearchChange(event.currentTarget.value)}
          onClear={() => onSearchChange("")}
          clearLabel={t("missions.clearSearch")}
          placeholder={t("missions.searchPlaceholder")}
          className="min-w-[220px] md:flex-1"
        />
        <FilterSelect value={statusFilter} onChange={onStatusFilterChange} label={t("missions.filter.status")}>
          {statusOptions.map((status) => (<option key={status.value} value={status.value}>{status.label}</option>))}
        </FilterSelect>
        <FilterSelect value={projectFilter} onChange={onProjectFilterChange} label={t("missions.filter.project")}>
          <option value="all">{t("missions.filter.allProjects")}</option>
          {projectOptions.map((project) => (<option key={project.id} value={project.id}>{project.name}</option>))}
        </FilterSelect>
      </Toolbar>

      <div className="mt-5 space-y-3">
        {missions.length
          ? missions.map((mission) => (
              <MissionRow
                key={mission.id}
                mission={mission}
                selectedMissionId={selectedMissionId}
                onSelectMission={onSelectMission}
                onOpenProject={onOpenProject}
              />
            ))
          : (
              <EmptyPanel
                title={t("missions.emptyTitle")}
                description={t("missions.emptyDesc")}
                variant="plain"
              />
            )}
      </div>
    </Card>
  );
}
