// @ts-nocheck
import React from "react";
import { Button } from "../../../design-system/button";
import { Card } from "../../../design-system/card";
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

// The header subtitle describes what the selected view IS — what these
// automations do — rather than reading back counts or states. One functional
// line per filter tab, switched as the filter changes.
const SUBTEXT_KEYS = {
  all: "automations.subtext.all",
  active: "automations.subtext.active",
  running: "automations.subtext.running",
  failures: "automations.subtext.failures",
  paused: "automations.subtext.paused",
  completed: "automations.subtext.completed",
};

const COLUMN_LABEL_CLASS =
  "font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]";

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

  const sortControl = (
    <SelectMenu
      value={sort}
      options={sortOptions}
      onChange={setSort}
      prefix={t("automations.sort.label")}
      ariaLabel={t("automations.sort.label")}
      className="min-w-[10rem]"
    />
  );

  return (
    <div className="space-y-5">
      <div className="min-w-0">
        <h2 className="text-[1.75rem] font-semibold tracking-tight text-[var(--v2-text-strong)]">
          {t("automations.title")}
        </h2>
        <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
          {t(SUBTEXT_KEYS[filter] ?? SUBTEXT_KEYS.all)}
        </p>
      </div>

      <AutomationsSummaryStrip summary={summary} nextRunAt={nextRunAt} />

      {!hasAutomations ? (
        <AutomationsEmptyState />
      ) : (
        <>
          {/* Mobile equivalent of the table-card header: the filter dropdown,
              sort, and delivery entrypoint live in their own flat card at the
              top of the card stack. */}
          <Card variant="flat" radius="sm" className="space-y-2 p-3 sm:hidden">
            <SelectMenu
              value={filter}
              options={filterOptions}
              onChange={onFilterChange}
              prefix={t("automations.filter.prefix")}
              ariaLabel={t("automations.filterLabel")}
              align="left"
              className="w-full"
              buttonClassName="w-full"
            />
            <div className="flex items-center gap-2">
              <SelectMenu
                value={sort}
                options={sortOptions}
                onChange={setSort}
                prefix={t("automations.sort.label")}
                ariaLabel={t("automations.sort.label")}
                align="left"
                className="min-w-0 flex-1"
                buttonClassName="w-full"
              />
              {deliveryState && (
                <Button
                  variant="secondary"
                  size="icon"
                  className="shrink-0"
                  aria-label={t("automations.delivery.setDefaults")}
                  title={t("automations.delivery.setDefaults")}
                  onClick={() => setDeliveryOpen(true)}
                >
                  <Icon name="gear" className="h-4 w-4" />
                </Button>
              )}
            </div>
          </Card>

          {/* The table card. Below sm it dissolves into the loose stack of
              row cards; from sm up it is one flat card whose header carries
              the filter tabs (left) and sort + delivery controls (right),
              with the column-label row beneath. */}
          <div
            className={cn(
              "flex flex-col gap-3",
              "sm:gap-0 sm:overflow-hidden sm:rounded-[1.25rem] md:rounded-[1.5rem]",
              "sm:border sm:border-[var(--v2-panel-border)] sm:bg-[var(--v2-card-bg)]"
            )}
          >
            {/* Card header: on lg+ tabs and controls share one baseline; below
                lg the controls sit on their own line above the tabs so the tab
                underline always merges with the header rule. */}
            <div className="hidden border-b border-[var(--v2-panel-border)] px-5 sm:flex sm:flex-col lg:flex-row lg:items-stretch lg:justify-between lg:gap-x-4">
              {/* The tab row stretches to the toolbar's full height so tab
                  labels sit vertically centered against the controls while
                  the active underline stays on the header hairline. */}
              <div className="order-2 flex min-w-0 items-stretch lg:order-1">
                <Tabs
                  bordered={false}
                  tabs={filterOptions}
                  value={filter}
                  onChange={onFilterChange}
                  ariaLabel={t("automations.filterLabel")}
                  className="min-w-0"
                />
              </div>
              <div className="order-1 flex items-center justify-end gap-2 pb-2 pt-2.5 lg:order-2 lg:py-2">
                {sortControl}
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

            {/* Column labels */}
            <div
              className={cn(
                ROW_COLUMNS.frame,
                "hidden border-b border-[var(--v2-panel-border)] px-5 py-2.5 sm:flex"
              )}
              aria-hidden="true"
            >
              <div className={cn(ROW_COLUMNS.name, COLUMN_LABEL_CLASS)}>
                {t("automations.table.name")}
              </div>
              <div className={cn(ROW_COLUMNS.schedule, COLUMN_LABEL_CLASS)}>
                {t("automations.table.schedule")}
              </div>
              <div className={cn(ROW_COLUMNS.status, COLUMN_LABEL_CLASS)}>
                {t("automations.table.status")}
              </div>
              <div className={cn(ROW_COLUMNS.lastRun, COLUMN_LABEL_CLASS)}>
                {t("automations.table.lastRun")}
              </div>
              <span className={ROW_COLUMNS.chevron} />
            </div>

            {filtered.length ? (
              filtered.map((automation) => (
                <AutomationRow
                  key={automation.automation_id}
                  automation={automation}
                  onOpen={setOpenId}
                />
              ))
            ) : (
              /* Filter matched nothing: keep the card (and its header, so the
                 filter stays reachable) and show the empty message inside. */
              <div className="rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] px-5 py-8 sm:rounded-none sm:border-0 sm:bg-transparent">
                <EmptyPanel
                  boxed={false}
                  title={t("automations.empty.matchingTitle")}
                  description={t("automations.empty.matchingDescription")}
                />
              </div>
            )}
          </div>
        </>
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
