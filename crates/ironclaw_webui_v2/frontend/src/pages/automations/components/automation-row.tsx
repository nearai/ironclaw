// @ts-nocheck
import { Icon } from "../../../design-system/icons";
import { StatusPill } from "../../../design-system/primitives";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
import { RunDots } from "./automation-recent-runs";

// Column geometry shared by the rows and the list's header-label row so the
// "Name / Status / Last run" headers line up exactly with the cells below.
// The name column owns the icon well, so the header's NAME label sits flush
// with the icon's left edge.
export const ROW_COLUMNS = {
  frame: "flex w-full items-center gap-3 sm:gap-4",
  name: "flex min-w-0 flex-1 items-center gap-3 sm:gap-4",
  status: "hidden w-28 shrink-0 sm:flex",
  lastRun: "hidden w-44 shrink-0 flex-col items-end md:flex",
  chevron: "hidden h-4 w-4 shrink-0 sm:block",
};

// A single automation in the list.
//
// Below sm it renders as its own card on the canvas (the list container adds
// no chrome there): icon + name/cadence with the status chip top-right, and a
// hairline footer carrying run history + last-run time. From sm up it becomes
// a table-like row inside the fused list panel under the list's column header
// labels: Name | Status | Last run (timestamp with the run-history dots
// stacked beneath it). The name column always keeps priority, so nothing
// clips at tablet widths.
export function AutomationRow({ automation, onOpen }) {
  const t = useT();

  const statusTone = automation.primary_status_tone;
  const statusLabel = automation.primary_status_label;

  return (
    <button
      type="button"
      onClick={() => onOpen(automation.automation_id)}
      className={cn(
        "group w-full text-left",
        // Mobile: stand-alone card, DS Card "default" surface.
        "rounded-[14px] border border-[var(--v2-card-border)] bg-[var(--v2-card-bg)] p-4 shadow-[var(--v2-card-shadow)]",
        // sm+: flat row inside the list panel with hairline dividers.
        "sm:rounded-none sm:border-0 sm:border-b sm:border-b-[var(--v2-panel-border)] sm:p-0 sm:shadow-none sm:last:border-b-0",
        "sm:hover:bg-[var(--v2-surface-soft)]",
        "focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--v2-accent)]"
      )}
    >
      <div className={cn(ROW_COLUMNS.frame, "sm:px-5 sm:py-3.5")}>
        <div className={ROW_COLUMNS.name}>
          <span
            className={cn(
              "grid h-10 w-10 shrink-0 place-items-center rounded-[var(--v2-radius-md)] border",
              "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text)]",
              "group-hover:border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))]"
            )}
          >
            <Icon name={automation.icon} className="h-[1.15rem] w-[1.15rem]" />
          </span>

          <div className="min-w-0 flex-1">
            <div
              className="truncate text-sm font-semibold text-[var(--v2-text-strong)]"
              title={automation.display_name}
            >
              {automation.display_name}
            </div>
            <div
              className="mt-0.5 truncate text-[13px] leading-5 text-[var(--v2-text-muted)]"
              title={automation.schedule_label}
            >
              {automation.schedule_label}
            </div>
          </div>

          {/* On mobile the status chip joins the card header row. */}
          <div className="flex shrink-0 justify-end sm:hidden">
            <StatusPill
              tone={statusTone}
              label={statusLabel}
              size="sm"
              className="whitespace-nowrap"
            />
          </div>
        </div>

        {/* Status — labelled column under the list's STATUS header */}
        <div className={ROW_COLUMNS.status}>
          <StatusPill
            tone={statusTone}
            label={statusLabel}
            size="sm"
            className="whitespace-nowrap"
          />
        </div>

        {/* Last run — timestamp with the run-history dots stacked beneath.
            Hovering the timestamp crossfades it to a relative "x ago". */}
        <div className={cn(ROW_COLUMNS.lastRun, "group/lastrun gap-1.5 text-right")}>
          <span className="relative block w-full text-[13px] leading-none text-[var(--v2-text)]">
            <span
              className={cn(
                "block truncate transition-opacity duration-200",
                automation.last_run_relative && "group-hover/lastrun:opacity-0"
              )}
              title={automation.last_run_label}
            >
              {automation.last_run_label}
            </span>
            {automation.last_run_relative && (
              <span
                aria-hidden="true"
                className="absolute inset-0 block truncate opacity-0 transition-opacity duration-200 group-hover/lastrun:opacity-100"
              >
                {automation.last_run_relative}
              </span>
            )}
          </span>
          <RunDots runs={automation.recent_runs} />
        </div>

        <Icon
          name="chevron"
          className={cn(
            ROW_COLUMNS.chevron,
            "-rotate-90 text-[var(--v2-text-faint)] group-hover:text-[var(--v2-text)]"
          )}
        />
      </div>

      {/* Mobile-only footer: run history + last run on one hairline row */}
      <div className="mt-3 flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] pt-3 sm:hidden">
        <RunDots runs={automation.recent_runs} />
        <span
          className="min-w-0 truncate text-xs text-[var(--v2-text-muted)]"
          title={automation.last_run_label}
        >
          {`${t("automations.row.lastRun")} · ${
            automation.last_run_relative || automation.last_run_label
          }`}
        </span>
      </div>
    </button>
  );
}
