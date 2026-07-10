import { Icon } from "../../../design-system/icons.js";
import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import { RunDots } from "./automation-recent-runs.js";

// A single automation in the list.
//
// Below sm it renders as its own card on the canvas (the list container adds
// no chrome there): icon + name/cadence with the status chip top-right, and a
// hairline footer carrying run history + last-run time. From sm up it becomes
// a full-width row inside the fused list panel; the run-dots and last-run
// columns fade in at lg/md respectively so nothing ever clips at tablet
// widths — the name column always keeps priority.
export function AutomationRow({ automation, onOpen }) {
  const t = useT();

  const statusTone = automation.primary_status_tone;
  const statusLabel = automation.primary_status_label;

  return html`
    <button
      type="button"
      onClick=${() => onOpen(automation.automation_id)}
      className=${cn(
        "group w-full text-left",
        // Mobile: stand-alone card, DS Card "default" surface.
        "rounded-[14px] border border-[var(--v2-card-border)] bg-[var(--v2-card-bg)] p-4 shadow-[var(--v2-card-shadow)]",
        // sm+: flat row inside the list panel with hairline dividers.
        "sm:rounded-none sm:border-0 sm:border-b sm:border-b-[var(--v2-panel-border)] sm:p-0 sm:shadow-none sm:last:border-b-0",
        "sm:hover:bg-[var(--v2-surface-soft)]",
        "focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--v2-accent)]"
      )}
    >
      <div className="flex w-full items-center gap-3 sm:gap-4 sm:px-5 sm:py-4">
        <span
          className=${cn(
            "grid h-10 w-10 shrink-0 place-items-center rounded-[var(--v2-radius-md)] border",
            "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text)]",
            "group-hover:border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))]"
          )}
        >
          <${Icon} name=${automation.icon} className="h-[1.15rem] w-[1.15rem]" />
        </span>

        <div className="min-w-0 flex-1">
          <div
            className="truncate text-sm font-semibold text-[var(--v2-text-strong)]"
            title=${automation.display_name}
          >
            ${automation.display_name}
          </div>
          <div
            className="mt-0.5 truncate text-[13px] leading-5 text-[var(--v2-text-muted)]"
            title=${automation.schedule_label}
          >
            ${automation.schedule_label}
          </div>
        </div>

        <!-- Run history (dots) — desktop-only column -->
        <div className="hidden w-32 shrink-0 justify-start lg:flex">
          <${RunDots} runs=${automation.recent_runs} />
        </div>

        <!-- Most-recent-run time — tablet-and-up column. Hovering the value
             crossfades the absolute time to a relative "x ago". -->
        <div
          className="group/lastrun hidden w-32 shrink-0 flex-col items-end gap-0.5 text-right md:flex"
        >
          <span
            className="font-mono text-[10px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]"
          >
            ${t("automations.row.lastRun")}
          </span>
          <span className="relative block w-full text-[13px] text-[var(--v2-text)]">
            <span
              className=${cn(
                "block truncate transition-opacity duration-200",
                automation.last_run_relative && "group-hover/lastrun:opacity-0"
              )}
              title=${automation.last_run_label}
            >
              ${automation.last_run_label}
            </span>
            ${automation.last_run_relative &&
            html`<span
              aria-hidden="true"
              className="absolute inset-0 block truncate opacity-0 transition-opacity duration-200 group-hover/lastrun:opacity-100"
            >
              ${automation.last_run_relative}
            </span>`}
          </span>
        </div>

        <!-- Status — right-aligned; part of the header row on mobile too -->
        <div className="flex shrink-0 justify-end sm:w-28">
          <${StatusPill}
            tone=${statusTone}
            label=${statusLabel}
            size="sm"
            className="whitespace-nowrap"
          />
        </div>

        <${Icon}
          name="chevron"
          className="hidden h-4 w-4 shrink-0 -rotate-90 text-[var(--v2-text-faint)] group-hover:text-[var(--v2-text)] sm:block"
        />
      </div>

      <!-- Mobile-only footer: run history + last run on one hairline row -->
      <div
        className="mt-3 flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] pt-3 sm:hidden"
      >
        <${RunDots} runs=${automation.recent_runs} />
        <span
          className="min-w-0 truncate text-xs text-[var(--v2-text-muted)]"
          title=${automation.last_run_label}
        >
          ${`${t("automations.row.lastRun")} · ${
            automation.last_run_relative || automation.last_run_label
          }`}
        </span>
      </div>
    </button>
  `;
}
