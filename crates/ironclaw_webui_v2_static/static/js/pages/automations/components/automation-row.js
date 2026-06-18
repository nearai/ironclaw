import { Icon } from "../../../design-system/icons.js";
import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import { RunDots } from "./automation-recent-runs.js";

// A single automation in the list. Left side carries the derived icon, title,
// and cadence description; the right side breaks the run history, the
// most-recent-run time, and the status into their own right-aligned columns so
// they line up cleanly down the list.
export function AutomationRow({ automation, onOpen }) {
  const t = useT();

  const statusTone = automation.has_running_run
    ? "info"
    : automation.has_failed_runs
      ? "danger"
      : automation.state_tone;
  const statusLabel = automation.has_running_run
    ? t("automations.status.running")
    : automation.has_failed_runs
      ? t("automations.status.needsReview")
      : automation.state_label;

  return html`
    <button
      type="button"
      onClick=${() => onOpen(automation.automation_id)}
      className=${cn(
        "group flex w-full items-center gap-4 px-4 py-4 text-left sm:px-5",
        "border-b border-[var(--v2-panel-border)] last:border-0",
        "hover:bg-white/[0.03] focus-visible:outline focus-visible:outline-2",
        "focus-visible:-outline-offset-2 focus-visible:outline-[var(--v2-accent)]"
      )}
    >
      <span
        className=${cn(
          "grid h-11 w-11 shrink-0 place-items-center rounded-[12px] border",
          "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-iron-200",
          "group-hover:border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))]"
        )}
      >
        <${Icon} name=${automation.icon} className="h-5 w-5" />
      </span>

      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-semibold text-iron-100">
          ${automation.display_name}
        </div>
        <div className="mt-1 truncate text-sm text-iron-300">
          ${automation.schedule_label}
        </div>
        <div className="mt-1.5 sm:hidden">
          <${StatusPill} tone=${statusTone} label=${statusLabel} />
        </div>
      </div>

      <!-- Run history (icons) — own left-aligned column -->
      <div className="hidden w-40 shrink-0 justify-start sm:flex">
        <${RunDots} runs=${automation.recent_runs} />
      </div>

      <!-- Most-recent-run time (details) — own right-aligned column. Hovering
           the value crossfades the absolute time to a relative "x ago". -->
      <div className="group/lastrun hidden w-36 shrink-0 flex-col items-end gap-0.5 text-right sm:flex">
        <span className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
          ${t("automations.row.lastRun")}
        </span>
        <span className="relative block w-full text-sm text-iron-200">
          <span
            className=${cn(
              "block truncate transition-opacity duration-200",
              automation.last_run_relative && "group-hover/lastrun:opacity-0"
            )}
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

      <!-- Status — own right-aligned column -->
      <div className="hidden w-32 shrink-0 justify-end sm:flex">
        <${StatusPill}
          tone=${statusTone}
          label=${statusLabel}
          className="whitespace-nowrap"
        />
      </div>

      <${Icon}
        name="chevron"
        className="hidden h-4 w-4 shrink-0 -rotate-90 text-iron-400 group-hover:text-iron-200 sm:block"
      />
    </button>
  `;
}
