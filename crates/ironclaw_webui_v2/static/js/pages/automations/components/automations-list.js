import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { EmptyPanel, Panel } from "../../../design-system/primitives.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import {
  AUTOMATION_FILTERS,
  AUTOMATION_SORTS,
  filterAutomations,
  sortAutomations,
} from "../lib/automations-presenters.js";
import { AutomationDeliveryDefaultsModal } from "./automation-delivery-defaults-modal.js";
import { AutomationDetailModal } from "./automation-detail-modal.js";
import { AutomationRow } from "./automation-row.js";
import { AutomationsEmptyState } from "./automations-empty-state.js";
import { AutomationsSummaryStrip } from "./automations-summary-strip.js";

export function AutomationsList({
  automations,
  summary,
  nextRunAt,
  filter,
  onFilterChange,
  deliveryState,
  isMutating,
  onPauseAutomation,
  onResumeAutomation,
  onDeleteAutomation,
}) {
  const t = useT();
  // Default sort mirrors the natural ordering (active first, soonest next run).
  const [sort, setSort] = React.useState("next");
  const filtered = sortAutomations(filterAutomations(automations, filter), sort);
  const hasAutomations = automations.length > 0;

  // The detail modal is opened by automation id (not index) so it survives
  // refetches that reorder or drop rows.
  const [openId, setOpenId] = React.useState(null);
  const openAutomation =
    automations.find((automation) => automation.automation_id === openId) || null;
  const [deliveryOpen, setDeliveryOpen] = React.useState(false);

  return html`
    <div className="space-y-5">
      <div className="mt-4 flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
        <div className="flex items-center gap-3">
          <h2 className="text-[1.75rem] font-semibold tracking-tight text-iron-100">
            ${t("automations.title")}
          </h2>
          <span
            className="h-6 w-px shrink-0 bg-[var(--v2-panel-border)]"
            aria-hidden="true"
          ></span>
          <p className="text-sm leading-6 text-iron-300">
            ${t("automations.description")}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2 lg:flex-nowrap">
          <div
            className="inline-flex h-9 max-w-full shrink-0 items-center gap-0.5 overflow-x-auto rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] p-0.5"
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
                  "shrink-0 whitespace-nowrap rounded-full px-2.5 py-1.5 text-[11px] font-medium leading-none transition-colors",
                  filter === item.value
                    ? "bg-[var(--v2-surface)] text-[var(--v2-text-strong)] shadow-sm"
                    : "text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
                )}
              >
                ${t(item.labelKey)}
              </button>
            `)}
          </div>
          <label
            className="inline-flex h-9 shrink-0 items-center rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] pl-3 focus-within:border-[var(--v2-accent)]"
          >
            <span className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
              ${t("automations.sort.label")}
            </span>
            <select
              value=${sort}
              onChange=${(event) => setSort(event.target.value)}
              aria-label=${t("automations.sort.label")}
              style=${{
                backgroundImage: "var(--v2-select-chevron)",
                backgroundRepeat: "no-repeat",
                backgroundPosition: "right 0.75rem center",
              }}
              className="h-full appearance-none rounded-full bg-transparent pl-2 pr-8 text-[11px] font-medium text-[var(--v2-text-strong)] focus-visible:outline-none"
            >
              ${AUTOMATION_SORTS.map(
                (item) => html`<option key=${item.value} value=${item.value}>
                  ${t(item.labelKey)}
                </option>`
              )}
            </select>
          </label>
          ${deliveryState &&
          html`
            <${Button}
              variant="secondary"
              size="sm"
              className="shrink-0"
              onClick=${() => setDeliveryOpen(true)}
            >
              <${Icon} name="gear" className="mr-1.5 h-4 w-4" />
              ${t("automations.delivery.setDefaults")}
            <//>
          `}
        </div>
      </div>

      <${AutomationsSummaryStrip} summary=${summary} nextRunAt=${nextRunAt} />

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
            <${Panel} className="overflow-hidden">
              <div className="flex flex-col">
                ${filtered.map(
                  (automation) => html`
                    <${AutomationRow}
                      key=${automation.automation_id}
                      automation=${automation}
                      onOpen=${setOpenId}
                    />
                  `
                )}
              </div>
            <//>
          `}

      <${AutomationDetailModal}
        automation=${openAutomation}
        open=${Boolean(openAutomation)}
        onClose=${() => setOpenId(null)}
        isMutating=${isMutating}
        onPauseAutomation=${onPauseAutomation}
        onResumeAutomation=${onResumeAutomation}
        onDeleteAutomation=${onDeleteAutomation}
      />

      ${deliveryState &&
      html`<${AutomationDeliveryDefaultsModal}
        deliveryState=${deliveryState}
        open=${deliveryOpen}
        onClose=${() => setDeliveryOpen(false)}
      />`}
    </div>
  `;
}
