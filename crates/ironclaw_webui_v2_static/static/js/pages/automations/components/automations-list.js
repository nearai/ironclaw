import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { EmptyPanel, Panel } from "../../../design-system/primitives.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import { AUTOMATION_FILTERS, filterAutomations } from "../lib/automations-presenters.js";
import { AutomationDeliveryDefaultsModal } from "./automation-delivery-defaults-modal.js";
import { AutomationDetailModal } from "./automation-detail-modal.js";
import { AutomationRow } from "./automation-row.js";
import { AutomationsEmptyState } from "./automations-empty-state.js";

export function AutomationsList({
  automations,
  filter,
  onFilterChange,
  onRefresh,
  isRefreshing,
  deliveryState,
}) {
  const t = useT();
  const filtered = filterAutomations(automations, filter);
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
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
            ${t("automations.eyebrow")}
          </div>
          <div className="mt-2 flex items-center gap-3">
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
          ${deliveryState &&
          html`
            <${Button}
              variant="secondary"
              size="sm"
              onClick=${() => setDeliveryOpen(true)}
            >
              <${Icon} name="settings" className="mr-1.5 h-4 w-4" />
              ${t("automations.delivery.setDefaults")}
            <//>
          `}
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
