import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { cn } from "../../../utils/cn.js";
import {
  AUTOMATION_FILTERS,
  filterAutomations,
} from "../lib/automations-presenters.js";

export function AutomationsList({
  automations,
  filter,
  onFilterChange,
  onRefresh,
  isRefreshing,
}) {
  const filtered = filterAutomations(automations, filter);

  return html`
    <div className="space-y-5">
      <${Panel} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              Scheduled work
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              Automations
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              Scheduled automations only.
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div
              className="inline-flex overflow-hidden rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
              role="group"
              aria-label="Automation status filter"
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
                  ${item.label}
                </button>
              `)}
            </div>
            <${Button}
              variant="secondary"
              size="icon-sm"
              aria-label="Refresh automations"
              disabled=${isRefreshing}
              onClick=${onRefresh}
            >
              <${Icon} name="retry" className="h-4 w-4" />
            <//>
          </div>
        </div>
      <//>

      ${!filtered.length
        ? html`
            <${EmptyPanel}
              title=${automations.length ? "No matching automations" : "No scheduled automations yet."}
              description=${automations.length
                ? "Try a different status filter."
                : "This agent has no scheduled work to show."}
            />
          `
        : html`
            <${Panel} className="overflow-hidden">
              <div className="overflow-x-auto">
                <table className="w-full min-w-[820px] border-collapse">
                  <thead>
                    <tr className="border-b border-[var(--v2-panel-border)] text-left">
                      <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                        Name
                      </th>
                      <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                        Schedule
                      </th>
                      <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                        Next run
                      </th>
                      <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                        Last run
                      </th>
                      <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                        Status
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    ${filtered.map((automation) => html`
                      <tr
                        key=${automation.automation_id}
                        className="border-b border-[var(--v2-panel-border)] last:border-0"
                      >
                        <td className="max-w-[280px] px-5 py-4 align-top">
                          <div className="truncate text-sm font-semibold text-iron-100">
                            ${automation.display_name}
                          </div>
                          <div className="mt-1 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
                            ${automation.automation_id}
                          </div>
                        </td>
                        <td className="px-5 py-4 align-top text-sm text-iron-200">
                          ${automation.schedule_label}
                        </td>
                        <td className="px-5 py-4 align-top text-sm text-iron-200">
                          ${automation.next_run_label}
                        </td>
                        <td className="px-5 py-4 align-top">
                          <div className="text-sm text-iron-200">
                            ${automation.last_run_label}
                          </div>
                          <div className="mt-2">
                            <${StatusPill}
                              tone=${automation.last_status_tone}
                              label=${automation.last_status_label}
                            />
                          </div>
                        </td>
                        <td className="px-5 py-4 align-top">
                          <${StatusPill}
                            tone=${automation.state_tone}
                            label=${automation.state_label}
                          />
                        </td>
                      </tr>
                    `)}
                  </tbody>
                </table>
              </div>
            <//>
          `}
    </div>
  `;
}
