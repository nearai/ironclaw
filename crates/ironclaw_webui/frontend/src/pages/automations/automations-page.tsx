import React from "react";
import { Callout, SkeletonList } from "@ironclaw/ui";
import { useT } from "../../lib/i18n";
import { AutomationDeliveryDefaultsPanel } from "./components/automation-delivery-defaults-panel";
import { AutomationsList } from "./components/automations-list";
import { AutomationsSummaryStrip } from "./components/automations-summary-strip";
import { useAutomations } from "./hooks/useAutomations";
import { useOutboundDeliveryDefaults } from "./hooks/useOutboundDeliveryDefaults";

export function AutomationsPage() {
  const t = useT();
  const [filter, setFilter] = React.useState("all");
  const [selectedAutomationId, setSelectedAutomationId] = React.useState(null);
  const includeCompleted = filter === "completed";
  const automationsState = useAutomations(includeCompleted);
  const deliveryState = useOutboundDeliveryDefaults();

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
            <Callout tone="danger">
              {t("automations.error.loadFailed")}
            </Callout>
          )}
          {showErrorOnly
            ? null
            : (
                <>
                {!automationsState.isLoading &&
                !automationsState.schedulerEnabled &&
                (
                  <Callout tone="warning" title={t("automations.schedulerOff.title")}>
                    {t("automations.schedulerOff.description")}
                  </Callout>
                )}
                <AutomationsSummaryStrip
                  summary={automationsState.summary}
                  activeFilter={filter}
                  onSelectFilter={setFilter}
                />
                <AutomationDeliveryDefaultsPanel deliveryState={deliveryState} />

                {automationsState.isLoading
                  ? (<SkeletonList label={t("automations.loading")} />)
                  : (
                      <AutomationsList
                        automations={automationsState.automations}
                        filter={filter}
                        onFilterChange={setFilter}
                        onRefresh={handleRefresh}
                        isRefreshing={isRefreshing}
                        isFilterTransition={automationsState.isFilterTransition}
                        isMutating={automationsState.isMutating}
                        selectedAutomationId={selectedAutomationId}
                        onSelectAutomation={setSelectedAutomationId}
                        onPauseAutomation={automationsState.pauseAutomation}
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
