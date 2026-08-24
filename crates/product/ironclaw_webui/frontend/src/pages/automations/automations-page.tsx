import React from "react";
import { useT } from "../../lib/i18n";
import { AutomationsList } from "./components/automations-list";
import { AutomationsSummaryStrip } from "./components/automations-summary-strip";
import { NotificationChannelsPanel } from "./components/notification-channels-panel";
import { useAutomations } from "./hooks/useAutomations";
import { useNotificationChannels } from "./hooks/useNotificationChannels";

export function AutomationsPage() {
  const t = useT();
  const [filter, setFilter] = React.useState("all");
  const [selectedAutomationId, setSelectedAutomationId] = React.useState(null);
  const includeCompleted = filter === "completed" || filter === "failures";
  // The hook fetches the lean scheduled list first and a completed-inclusive
  // summary in parallel. Switching to either historical filter then reuses
  // that full query while keepPreviousData preserves the visible rows.
  const automationsState = useAutomations(includeCompleted);
  const channelsState = useNotificationChannels();

  // A local refetch can resolve almost instantly, leaving the spinner to flash
  // imperceptibly. Hold a minimum spin window so a manual refresh always reads
  // as a deliberate action.
  const [minSpin, setMinSpin] = React.useState(false);
  const minSpinTimer = React.useRef(null);
  React.useEffect(() => () => clearTimeout(minSpinTimer.current), []);
  const handleRefresh = React.useCallback(() => {
    setMinSpin(true);
    clearTimeout(minSpinTimer.current);
    minSpinTimer.current = setTimeout(() => setMinSpin(false), 1000);
    automationsState.refetch();
  }, [automationsState.refetch]);
  const isRefreshing = automationsState.isRefreshing || minSpin;
  const showErrorOnly =
    automationsState.error &&
    !automationsState.isLoading &&
    automationsState.automations.length === 0;

  React.useEffect(() => {
    // Placeholder rows belong to the previous filter. Preserve their current
    // selection until the new query payload arrives instead of choosing a row
    // under a filter whose data is not available yet.
    if (automationsState.isFilterTransition) return;
    if (!automationsState.automations.length) {
      setSelectedAutomationId(null);
      return;
    }
    const stillExists = automationsState.automations.some(
      (automation) => automation.automation_id === selectedAutomationId
    );
    if (!stillExists) {
      setSelectedAutomationId(automationsState.automations[0].automation_id);
    }
  }, [
    automationsState.automations,
    automationsState.isFilterTransition,
    selectedAutomationId,
  ]);

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          {automationsState.error &&
          (
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              {t("automations.error.loadFailed")}
            </div>
          )}
          {!automationsState.error && automationsState.summaryError &&
          (
            <div
              role="status"
              className="rounded-xl border border-amber-400/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-200"
            >
              {t("automations.error.loadFailed")}
            </div>
          )}
          {showErrorOnly
            ? null
            : (
                <>
                {!automationsState.isLoading &&
                !automationsState.schedulerEnabled &&
                (
                  <div
                    role="status"
                    className="rounded-xl border border-amber-400/30 bg-amber-500/10 px-4 py-3"
                  >
                    <div className="text-sm font-semibold text-amber-200">
                      {t("automations.schedulerOff.title")}
                    </div>
                    <div className="mt-0.5 text-xs leading-5 text-amber-200/80">
                      {t("automations.schedulerOff.description")}
                    </div>
                  </div>
                )}
                <AutomationsSummaryStrip
                  summary={automationsState.summary}
                  activeFilter={filter}
                  onSelectFilter={setFilter}
                />
                <NotificationChannelsPanel channelsState={channelsState} />

                {automationsState.isLoading
                  ? (
                      <div className="space-y-4">
                        {[1, 2, 3].map(
                          (index) =>
                            (<div
                              key={index}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />)
                        )}
                      </div>
                    )
                  : (
                      <AutomationsList
                        automations={automationsState.automations}
                        filter={filter}
                        onFilterChange={setFilter}
                        onRefresh={handleRefresh}
                        isRefreshing={isRefreshing}
                        isFilterTransition={automationsState.isFilterTransition}
                        isMutating={automationsState.isMutating}
                        schedulerEnabled={automationsState.schedulerEnabled}
                        selectedAutomationId={selectedAutomationId}
                        onSelectAutomation={setSelectedAutomationId}
                        onPauseAutomation={automationsState.pauseAutomation}
                        onRunAutomation={automationsState.runAutomation}
                        onResumeAutomation={automationsState.resumeAutomation}
                        onRenameAutomation={automationsState.renameAutomation}
                        onDeleteAutomation={automationsState.deleteAutomation}
                      />
                    )}
                </>
              )}
        </div>
      </div>
    </div>
  );
}
