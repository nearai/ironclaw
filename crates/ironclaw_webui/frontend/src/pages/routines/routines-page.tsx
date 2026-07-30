import { useNavigate, useParams } from "react-router";
import { Button, Callout, SkeletonList } from "@ironclaw/ui";
import React from "react";
import { useT } from "../../lib/i18n";
import { FeedbackBanner } from "../projects/components/feedback-banner";
import { RoutineDetailPanel } from "./components/routine-detail-panel";
import { RoutinesList } from "./components/routines-list";
import { RoutinesSummaryStrip } from "./components/routines-summary-strip";
import { useRoutineFilters } from "./hooks/useRoutineFilters";
import { useRoutineDetail } from "./hooks/useRoutineDetail";
import { useRoutines } from "./hooks/useRoutines";

export function RoutinesPage() {
  const t = useT();
  const navigate = useNavigate();
  const { routineId = null } = useParams();
  const routinesState = useRoutines();
  const detailState = useRoutineDetail(routineId);
  const filters = useRoutineFilters(routinesState.routines);

  const handleRoutineAction = React.useCallback(async (action, targetId) => {
    try {
      await action({ routineId: targetId });
    } catch {
      // Mutation hooks own the visible result state.
    }
  }, []);

  const handleDelete = React.useCallback(
    async (targetId, name) => {
      if (!window.confirm(`Delete routine "${name}"?`)) return;
      try {
        await routinesState.deleteRoutine({ routineId: targetId });
        navigate("/routines");
      } catch {
        // Mutation hooks own the visible result state.
      }
    },
    [navigate, routinesState]
  );

  const detailContent = routineId
    ? (
        <div className="grid gap-5 xl:grid-cols-[minmax(0,0.9fr)_minmax(440px,1.1fr)]">
          <RoutinesList
            routines={filters.filteredRoutines}
            totalRoutines={routinesState.routines.length}
            selectedRoutineId={routineId}
            search={filters.search}
            onSearchChange={filters.setSearch}
            statusFilter={filters.statusFilter}
            onStatusFilterChange={filters.setStatusFilter}
            onSelectRoutine={(nextId) => navigate(`/routines/${nextId}`)}
            onTriggerRoutine={(nextId) =>
              handleRoutineAction(routinesState.triggerRoutine, nextId)}
            onToggleRoutine={(nextId) =>
              handleRoutineAction(routinesState.toggleRoutine, nextId)}
            isBusy={routinesState.isBusy}
            isRefreshing={routinesState.isRefreshing}
          />
          <RoutineDetailPanel
            routine={detailState.routine}
            isLoading={detailState.isLoading}
            error={detailState.error}
            isBusy={detailState.isBusy}
            onTriggerRoutine={detailState.triggerRoutine}
            onToggleRoutine={detailState.toggleRoutine}
            onDeleteRoutine={() =>
              handleDelete(routineId, detailState.routine?.name || routineId)}
          />
        </div>
      )
    : (
        <RoutinesList
          routines={filters.filteredRoutines}
          totalRoutines={routinesState.routines.length}
          selectedRoutineId={routineId}
          search={filters.search}
          onSearchChange={filters.setSearch}
          statusFilter={filters.statusFilter}
          onStatusFilterChange={filters.setStatusFilter}
          onSelectRoutine={(nextId) => navigate(`/routines/${nextId}`)}
          onTriggerRoutine={(nextId) =>
            handleRoutineAction(routinesState.triggerRoutine, nextId)}
          onToggleRoutine={(nextId) =>
            handleRoutineAction(routinesState.toggleRoutine, nextId)}
          isBusy={routinesState.isBusy}
          isRefreshing={routinesState.isRefreshing}
        />
      );

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          {routineId &&
          (<div className="flex flex-wrap justify-end gap-2">
            <Button variant="ghost" onClick={() => navigate("/routines")}>
              All routines
            </Button>
          </div>)}

          {routinesState.error &&
          (
            <Callout tone="danger">
              {routinesState.error.message}
            </Callout>
          )}

          <FeedbackBanner
            result={routinesState.actionResult}
            onDismiss={routinesState.clearActionResult}
          />
          <FeedbackBanner
            result={detailState.actionResult}
            onDismiss={detailState.clearActionResult}
          />
          <RoutinesSummaryStrip summary={routinesState.summary} />

          {routinesState.isLoading
            ? (<SkeletonList label={t("routines.loading")} itemClassName="h-32 rounded-xl" />)
            : detailContent}
        </div>
      </div>
    </div>
  );
}
