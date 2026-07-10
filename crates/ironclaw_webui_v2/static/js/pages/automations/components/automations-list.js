import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { Select } from "../../../design-system/input.js";
import { EmptyPanel } from "../../../design-system/primitives.js";
import { SegmentedControl } from "../../../design-system/segmented.js";
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

// The header subtitle is contextual: it describes the currently selected
// filter tab instead of repeating a static tagline. "All" gets a count-aware
// line; the status filters reuse the same localized descriptions the summary
// cards show, so both surfaces can never disagree.
function filterSubtext(filter, summary, t) {
  if (filter === "active") return t("automations.summary.activeDetail");
  if (filter === "running") return t("automations.summary.runningDetail");
  if (filter === "failures") return t("automations.summary.failuresDetail");
  if (filter === "paused") return t("automations.summary.pausedDetail");
  if (filter === "completed") return t("automations.subtext.completed");
  const count = summary?.scheduled ?? 0;
  const active = summary?.active ?? 0;
  return count === 1
    ? t("automations.subtext.one", { active })
    : t("automations.subtext.all", { count, active });
}

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

  const filterOptions = AUTOMATION_FILTERS.map((item) => ({
    value: item.value,
    label: t(item.labelKey),
  }));

  return html`
    <div className="space-y-5">
      <div
        className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between"
      >
        <div className="min-w-0">
          <h2
            className="text-[1.75rem] font-semibold tracking-tight text-[var(--v2-text-strong)]"
          >
            ${t("automations.title")}
          </h2>
          <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
            ${filterSubtext(filter, summary, t)}
          </p>
        </div>

        <!-- Toolbar: every control sits on the shared 32px control row
             (--v2-control-h-md) so the segmented filter, sort select, and
             button align exactly. Wraps to extra lines on narrow screens
             instead of clipping. -->
        <div className="flex max-w-full flex-wrap items-center gap-2">
          <${SegmentedControl}
            options=${filterOptions}
            value=${filter}
            onChange=${onFilterChange}
            ariaLabel=${t("automations.filterLabel")}
          />
          <label className="flex shrink-0 items-center gap-1.5">
            <span className="text-xs font-medium text-[var(--v2-text-muted)]">
              ${t("automations.sort.label")}
            </span>
            <${Select}
              size="sm"
              wrapperClassName="w-auto shrink-0"
              value=${sort}
              onChange=${(event) => setSort(event.target.value)}
              aria-label=${t("automations.sort.label")}
            >
              ${AUTOMATION_SORTS.map(
                (item) => html`<option key=${item.value} value=${item.value}>
                  ${t(item.labelKey)}
                </option>`
              )}
            <//>
          </label>
          ${deliveryState &&
          html`
            <${Button}
              variant="secondary"
              size="md"
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
            <!-- Below sm each automation is its own card on the canvas; from
                 sm up the rows fuse into one Card-styled panel with hairline
                 dividers (classes mirror the DS Card "default" variant). -->
            <div
              className=${cn(
                "flex flex-col gap-3",
                "sm:gap-0 sm:overflow-hidden sm:rounded-[1.25rem] md:rounded-[1.5rem]",
                "sm:border sm:border-[var(--v2-card-border)] sm:bg-[var(--v2-card-bg)]",
                "sm:shadow-[var(--v2-card-shadow)]"
              )}
            >
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
