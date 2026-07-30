import {
  Button,
  EmptyPanel,
  Icon,
  Card,
  SectionHeader,
  SegmentedControl,
  Badge,
  cn,
} from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";
import { AUTOMATION_FILTERS, filterAutomations } from "../lib/automations-presenters";
import { AutomationDetailPanel } from "./automation-detail-panel";
import { AutomationsEmptyState } from "./automations-empty-state";
import { RunDots, RunHistorySummary } from "./automation-recent-runs";

export function AutomationsList({
  automations,
  filter,
  onFilterChange,
  onRefresh,
  isRefreshing,
  isFilterTransition,
  isMutating,
  selectedAutomationId,
  onSelectAutomation,
  onPauseAutomation,
  onResumeAutomation,
  onRenameAutomation,
  onDeleteAutomation,
}) {
  const t = useT();
  const filtered = isFilterTransition
    ? automations
    : filterAutomations(automations, filter);
  const hasAutomations = automations.length > 0;
  const selectedAutomation =
    filtered.find((automation) => automation.automation_id === selectedAutomationId) ||
    (isFilterTransition ? null : filtered[0]) ||
    null;

  return (
    <div className="space-y-5">
      <Card className="p-4 sm:p-5">
        <SectionHeader
          eyebrow={t("automations.eyebrow")}
          title={t("automations.title")}
          description={t("automations.description")}
          actions={
            <>
              <SegmentedControl
                label={t("automations.filterLabel")}
                optionTestId="automation-filter"
                options={AUTOMATION_FILTERS.map((item) => ({
                  value: item.value,
                  label: t(item.labelKey),
                }))}
                value={filter}
                onChange={onFilterChange}
              />
              <Button
                variant="secondary"
                size="icon-sm"
                aria-label={t("automations.refresh")}
                title={isRefreshing ? t("automations.refreshing") : t("automations.refresh")}
                disabled={isRefreshing}
                onClick={onRefresh}
              >
                <Icon
                  name="retry"
                  className={cn("h-4 w-4", isRefreshing && "v2-spin")}
                />
              </Button>
            </>
          }
        />
      </Card>

      {!filtered.length
        ? hasAutomations
          ? (
              <EmptyPanel
                title={t("automations.empty.matchingTitle")}
                description={t("automations.empty.matchingDescription")}
              />
            )
          : (<AutomationsEmptyState />)
        : (
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <Card className="overflow-hidden">
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[900px] border-collapse">
                    <thead>
                      <tr className="border-b border-[var(--v2-panel-border)] text-left">
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          {t("automations.table.name")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          {t("automations.table.schedule")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          {t("automations.table.nextRun")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          {t("automations.table.recentRuns")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          {t("automations.table.status")}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {filtered.map((automation) => {
                        const selected =
                          automation.automation_id === selectedAutomation?.automation_id;
                        return (
                          <tr
                            key={automation.automation_id}
                            data-testid="automation-row"
                            data-automation-id={automation.automation_id}
                            className={cn(
                              "border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",
                              selected && "bg-[var(--v2-accent-soft)]/30"
                            )}
                          >
                            <td className="max-w-[280px] px-5 py-4 align-top">
                              <button
                                type="button"
                                aria-pressed={selected}
                                data-testid="automation-name-button"
                                data-automation-id={automation.automation_id}
                                onClick={() => onSelectAutomation(automation.automation_id)}
                                className="block w-full min-w-0 rounded text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]"
                              >
                                <div className="truncate text-sm font-semibold text-iron-100">
                                  {automation.display_name}
                                </div>
                                <div className="mt-1 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
                                  {automation.automation_id}
                                </div>
                              </button>
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              {automation.schedule_label}
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              {automation.next_run_label}
                            </td>
                            <td className="px-5 py-4 align-top">
                              <div className="space-y-2">
                                <RunDots runs={automation.recent_runs} />
                                <RunHistorySummary runs={automation.recent_runs} />
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top">
                              <Badge
                                tone={automation.primary_status_tone}
                                label={automation.primary_status_label}
                              />
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              </Card>

              <AutomationDetailPanel
                automation={selectedAutomation}
                isMutating={isMutating}
                onPauseAutomation={onPauseAutomation}
                onResumeAutomation={onResumeAutomation}
                onRenameAutomation={onRenameAutomation}
                onDeleteAutomation={onDeleteAutomation}
              />
            </div>
          )}
    </div>
  );
}
