import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import { AUTOMATION_FILTERS, filterAutomations } from "../lib/automations-presenters.js";
import { AutomationDetailPanel } from "./automation-detail-panel.js";
import { AutomationsEmptyState } from "./automations-empty-state.js";
import { RunDots, RunHistorySummary } from "./automation-recent-runs.js";

export function AutomationsList({
  automations,
  filter,
  onFilterChange,
  onRefresh,
  isRefreshing,
  isMutating,
  selectedAutomationId,
  onSelectAutomation,
  onPauseAutomation,
  onResumeAutomation,
}) {
  const t = useT();
  const filtered = filterAutomations(automations, filter);
  const hasAutomations = automations.length > 0;
  const selectedAutomation =
    filtered.find((automation) => automation.automation_id === selectedAutomationId) ||
    filtered[0] ||
    null;

  return html`
    <div className="space-y-5">
      <${Panel} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              ${t("automations.eyebrow")}
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              ${t("automations.title")}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${t("automations.description")}
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div
              className="inline-flex overflow-hidden rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
              role="group"
              aria-label=${t("automations.filterLabel")}
            >
              ${AUTOMATION_FILTERS.map((item) => html`
                <button
                  key=${item.value}
                  type="button"
                  aria-pressed=${filter === item.value}
                  onClick=${() => onFilterChange(item.value)}
                  className=${cn(
                    "h-9 px-3 text-xs font-semibold",
                    filter === item.value
                      ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                      : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
                  )}
                >
                  ${t(item.labelKey)}
                </button>
              `)}
            </div>
            <${Button}
              variant="secondary"
              size="icon-sm"
              aria-label=${t("automations.refresh")}
              title=${isRefreshing ? t("automations.refreshing") : t("automations.refresh")}
              disabled=${isRefreshing}
              onClick=${onRefresh}
            >
              <${Icon}
                name="retry"
                className=${cn("h-4 w-4", isRefreshing && "v2-spin")}
              />
            <//>
          </div>
        </div>
      <//>

      ${!filtered.length
        ? hasAutomations
          ? html`
              <${EmptyPanel}
                title=${t("automations.empty.matchingTitle")}
                description=${t("automations.empty.matchingDescription")}
              />
            `
          : html`<${AutomationsEmptyState} />`
        : html`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${Panel} className="overflow-hidden">
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[900px] border-collapse">
                    <thead>
                      <tr className="border-b border-[var(--v2-panel-border)] text-left">
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.name")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.schedule")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.nextRun")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.recentRuns")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.status")}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      ${filtered.map((automation) => {
                        const selected =
                          automation.automation_id === selectedAutomation?.automation_id;
                        return html`
                          <tr
                            key=${automation.automation_id}
                            className=${cn(
                              "border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03]",
                              selected && "bg-[var(--v2-accent-soft)]/30"
                            )}
                          >
                            <td className="max-w-[280px] px-5 py-4 align-top">
                              <button
                                type="button"
                                aria-pressed=${selected}
                                onClick=${() => onSelectAutomation(automation.automation_id)}
                                className="block w-full min-w-0 rounded text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]"
                              >
                                <div className="truncate text-sm font-semibold text-iron-100">
                                  ${automation.display_name}
                                </div>
                                <div className="mt-1 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
                                  ${automation.automation_id}
                                </div>
                              </button>
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              ${automation.schedule_label}
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              ${automation.next_run_label}
                            </td>
                            <td className="px-5 py-4 align-top">
                              <div className="space-y-2">
                                <${RunDots} runs=${automation.recent_runs} />
                                <${RunHistorySummary} runs=${automation.recent_runs} />
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${StatusPill}
                                tone=${automation.primary_status_tone}
                                label=${automation.primary_status_label}
                              />
                            </td>
                          </tr>
                        `;
                      })}
                    </tbody>
                  </table>
                </div>
              <//>

              <${AutomationDetailPanel}
                automation=${selectedAutomation}
                isMutating=${isMutating}
                onPauseAutomation=${onPauseAutomation}
                onResumeAutomation=${onResumeAutomation}
              />
            </div>
          `}
    </div>
  `;
}
