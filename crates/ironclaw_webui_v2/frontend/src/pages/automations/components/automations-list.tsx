// @ts-nocheck
import React from "react";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { EmptyPanel } from "../../../design-system/primitives";
import { SelectMenu } from "../../../design-system/select-menu";
import { Tabs } from "../../../design-system/tabs";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
import {
  AUTOMATION_FILTERS,
  AUTOMATION_SORTS,
  filterAutomations,
  sortAutomations,
} from "../lib/automations-presenters";
import { AutomationDeliveryDefaultsModal } from "./automation-delivery-defaults-modal";
import { AutomationDetailModal } from "./automation-detail-modal";
import { AutomationRow, ROW_COLUMNS } from "./automation-row";
import { AutomationsEmptyState } from "./automations-empty-state";
import { AutomationsSummaryStrip } from "./automations-summary-strip";

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
  onRenameAutomation,
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
  const sortOptions = AUTOMATION_SORTS.map((item) => ({
    value: item.value,
    label: t(item.labelKey),
  }));

  return (
    <div className="space-y-5">
      <div className="min-w-0">
        <h2 className="text-[1.75rem] font-semibold tracking-tight text-[var(--v2-text-strong)]">
          {t("automations.title")}
        </h2>
        <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
          {filterSubtext(filter, summary, t)}
        </p>
      </div>

      <AutomationsSummaryStrip summary={summary} nextRunAt={nextRunAt} />

      {/* Toolbar. Desktop/tablet: underline filter tabs sharing one baseline
          with the sort select and delivery-defaults button. Mobile swaps the
          tab row for a dropdown — no shrunken or scrolling tab strips. */}
      <div className="hidden flex-wrap items-end justify-between gap-x-4 gap-y-2 border-b border-[var(--v2-panel-border)] sm:flex">
        <Tabs
          bordered={false}
          tabs={filterOptions}
          value={filter}
          onChange={onFilterChange}
          ariaLabel={t("automations.filterLabel")}
          className="min-w-0"
        />
        <div className="flex items-center gap-2 pb-2">
          <label className="flex items-center gap-1.5">
            <span className="text-xs font-medium text-[var(--v2-text-muted)]">
              {t("automations.sort.label")}
            </span>
            <SelectMenu
              value={sort}
              options={sortOptions}
              onChange={setSort}
              ariaLabel={t("automations.sort.label")}
              className="min-w-[8.5rem]"
            />
          </label>
          {deliveryState && (
            <Button
              variant="secondary"
              size="md"
              className="shrink-0"
              onClick={() => setDeliveryOpen(true)}
            >
              <Icon name="gear" className="mr-1.5 h-4 w-4" />
              {t("automations.delivery.setDefaults")}
            </Button>
          )}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 sm:hidden">
        <SelectMenu
          value={filter}
          options={filterOptions}
          onChange={onFilterChange}
          ariaLabel={t("automations.filterLabel")}
          align="left"
          className="min-w-0 flex-1"
          buttonClassName="w-full"
        />
        <SelectMenu
          value={sort}
          options={sortOptions}
          onChange={setSort}
          ariaLabel={t("automations.sort.label")}
          className="min-w-0 flex-1"
          buttonClassName="w-full"
        />
        {deliveryState && (
          <Button
            variant="secondary"
            size="md"
            className="shrink-0"
            onClick={() => setDeliveryOpen(true)}
          >
            <Icon name="gear" className="mr-1.5 h-4 w-4" />
            {t("automations.delivery.setDefaults")}
          </Button>
        )}
      </div>

      {!filtered.length ? (
        hasAutomations ? (
          <EmptyPanel
            title={t("automations.empty.matchingTitle")}
            description={t("automations.empty.matchingDescription")}
          />
        ) : (
          <AutomationsEmptyState />
        )
      ) : (
        /* Below sm each automation is its own card on the canvas; from sm up
           the rows fuse into one Card-styled panel with a column-header label
           row (Name / Status / Last run) and hairline dividers. */
        <div
          className={cn(
            "flex flex-col gap-3",
            "sm:gap-0 sm:overflow-hidden sm:rounded-[1.25rem] md:rounded-[1.5rem]",
            "sm:border sm:border-[var(--v2-card-border)] sm:bg-[var(--v2-card-bg)]",
            "sm:shadow-[var(--v2-card-shadow)]"
          )}
        >
          <div
            className={cn(
              ROW_COLUMNS.frame,
              "hidden border-b border-[var(--v2-panel-border)] px-5 py-2.5 sm:flex"
            )}
            aria-hidden="true"
          >
            <div
              className={cn(
                ROW_COLUMNS.name,
                "font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]"
              )}
            >
              {t("automations.table.name")}
            </div>
            <div
              className={cn(
                ROW_COLUMNS.status,
                "font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]"
              )}
            >
              {t("automations.table.status")}
            </div>
            <div
              className={cn(
                ROW_COLUMNS.lastRun,
                "font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]"
              )}
            >
              {t("automations.table.lastRun")}
            </div>
            <span className={ROW_COLUMNS.chevron} />
          </div>
          {filtered.map((automation) => (
            <AutomationRow
              key={automation.automation_id}
              automation={automation}
              onOpen={setOpenId}
            />
          ))}
        </div>
      )}

      <AutomationDetailModal
        automation={openAutomation}
        open={Boolean(openAutomation)}
        onClose={() => setOpenId(null)}
        isMutating={isMutating}
        onPauseAutomation={onPauseAutomation}
        onResumeAutomation={onResumeAutomation}
        onRenameAutomation={onRenameAutomation}
        onDeleteAutomation={onDeleteAutomation}
      />

      {deliveryState && (
        <AutomationDeliveryDefaultsModal
          deliveryState={deliveryState}
          open={deliveryOpen}
          onClose={() => setDeliveryOpen(false)}
        />
      )}
    </div>
  );
}
