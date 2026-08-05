import { useNavigate, useParams } from "react-router";
import { Button } from "../../design-system/button";
import React from "react";
import { useT } from "../../lib/i18n";
import { FeedbackBanner } from "../projects/components/feedback-banner";
import { MissionDetailPanel } from "./components/mission-detail-panel";
import { MissionsList } from "./components/missions-list";
import { MissionsSummaryStrip } from "./components/missions-summary-strip";
import { useMissionDetail } from "./hooks/useMissionDetail";
import { useMissions } from "./hooks/useMissions";
import { sortMissions } from "./lib/missions-presenters";

export function MissionsPage() {
  const t = useT();
  const navigate = useNavigate();
  const { missionId = null } = useParams();
  const [search, setSearch] = React.useState("");
  const [statusFilter, setStatusFilter] = React.useState("all");
  const [projectFilter, setProjectFilter] = React.useState("all");

  const missionsState = useMissions();
  const detailState = useMissionDetail(missionId);

  const filteredMissions = React.useMemo(() => {
    const query = search.trim().toLowerCase();
    return sortMissions(missionsState.missions).filter((mission) => {
      const matchesSearch =
        !query ||
        [mission.name, mission.goal, mission.project?.name].some((value) =>
          String(value || "")
            .toLowerCase()
            .includes(query)
        );
      const matchesStatus =
        statusFilter === "all" || mission.status === statusFilter;
      const matchesProject =
        projectFilter === "all" || mission.project?.id === projectFilter;
      return matchesSearch && matchesStatus && matchesProject;
    });
  }, [missionsState.missions, projectFilter, search, statusFilter]);

  const listedMission = React.useMemo(
    () =>
      missionsState.missions.find((mission) => mission.id === missionId) ||
      null,
    [missionId, missionsState.missions]
  );

  const selectedMission = detailState.mission
    ? {
        ...listedMission,
        ...detailState.mission,
        project: listedMission?.project || null,
      }
    : listedMission;

  const handleOpenThread = React.useCallback(
    (thread) => {
      if (thread.project_id) {
        navigate(`/projects/${thread.project_id}/threads/${thread.id}`);
      }
    },
    [navigate]
  );

  const handleMissionAction = React.useCallback(
    async (action, targetMissionId) => {
      try {
        await action({ missionId: targetMissionId });
      } catch {
        // Mutation hooks own the visible result state.
      }
    },
    []
  );

  const content = missionId
    ? (
        <div
          className="grid gap-5 xl:grid-cols-[minmax(0,0.95fr)_minmax(420px,1.05fr)]"
        >
          <MissionsList
            missions={filteredMissions}
            totalMissions={missionsState.missions.length}
            selectedMissionId={missionId}
            search={search}
            onSearchChange={setSearch}
            statusFilter={statusFilter}
            onStatusFilterChange={setStatusFilter}
            projectFilter={projectFilter}
            onProjectFilterChange={setProjectFilter}
            projectOptions={missionsState.projects}
            onSelectMission={(nextMissionId) =>
              navigate(`/missions/${nextMissionId}`)}
            onOpenProject={(projectId) => navigate(`/projects/${projectId}`)}
          />
          <MissionDetailPanel
            mission={selectedMission}
            isLoading={detailState.isLoading}
            error={detailState.error}
            isBusy={missionsState.isBusy}
            onFire={(targetMissionId) =>
              handleMissionAction(missionsState.fireMission, targetMissionId)}
            onPause={(targetMissionId) =>
              handleMissionAction(missionsState.pauseMission, targetMissionId)}
            onResume={(targetMissionId) =>
              handleMissionAction(missionsState.resumeMission, targetMissionId)}
            onOpenProject={(projectId) => navigate(`/projects/${projectId}`)}
            onOpenThread={handleOpenThread}
          />
        </div>
      )
    : (
        <MissionsList
          missions={filteredMissions}
          totalMissions={missionsState.missions.length}
          selectedMissionId={missionId}
          search={search}
          onSearchChange={setSearch}
          statusFilter={statusFilter}
          onStatusFilterChange={setStatusFilter}
          projectFilter={projectFilter}
          onProjectFilterChange={setProjectFilter}
          projectOptions={missionsState.projects}
          onSelectMission={(nextMissionId) =>
            navigate(`/missions/${nextMissionId}`)}
          onOpenProject={(projectId) => navigate(`/projects/${projectId}`)}
        />
      );

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          {missionId &&
          (<div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="ghost"
              onClick={() => navigate("/missions")}
              >{t("missions.allMissions")}</Button>
          </div>)}

          {missionsState.error &&
          (
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              {missionsState.error.message}
            </div>
          )}

          <FeedbackBanner
            result={missionsState.actionResult}
            onDismiss={missionsState.clearActionResult}
          />
          <MissionsSummaryStrip summary={missionsState.summary} />

          {missionsState.isLoading
            ? (
                <div className="space-y-4">
                  {[1, 2, 3].map(
                    (index) =>
                      (<div
                        key={index}
                        className="v2-skeleton h-32 rounded-xl"
                      />)
                  )}
                </div>
              )
            : content}
        </div>
      </div>
    </div>
  );
}
