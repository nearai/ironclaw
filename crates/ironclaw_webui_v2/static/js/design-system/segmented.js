/**
 * SegmentedControl
 *
 * A single-select segmented strip (filter tabs / view switcher).  All
 * geometry comes from the shared control tokens so a segmented control
 * lines up with Button and Select in mixed toolbar rows:
 *   md → --v2-control-h-md (32px), --v2-radius-md
 *   sm → --v2-control-h-sm (28px), --v2-radius-sm
 *
 * When there is not enough room the segments wrap onto extra lines
 * inside the recessed container instead of clipping, so every option
 * stays reachable at any viewport width.
 *
 * Props
 *   options    [{ value, label }]
 *   value      currently selected option value
 *   onChange   (value) => void
 *   size       "sm" | "md" (default)
 *   ariaLabel  accessible name for the group
 *   className  layout additions for the container
 */
import { html } from "../lib/html.js";
import { cn } from "../utils/cn.js";

const CONTAINER_SIZES = {
  sm: "min-h-[var(--v2-control-h-sm)] rounded-[var(--v2-radius-sm)] p-[2px]",
  md: "min-h-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)] p-[3px]",
};

const SEGMENT_SIZES = {
  sm: "h-[calc(var(--v2-control-h-sm)-4px)] rounded-[calc(var(--v2-radius-sm)-2px)] px-2 text-[11px]",
  md: "h-[calc(var(--v2-control-h-md)-6px)] rounded-[calc(var(--v2-radius-md)-3px)] px-[var(--v2-control-px-sm)] text-xs",
};

export function SegmentedControl({
  options = [],
  value,
  onChange,
  size = "md",
  ariaLabel,
  className = "",
}) {
  return html`
    <div
      role="group"
      aria-label=${ariaLabel}
      className=${cn(
        "inline-flex max-w-full shrink flex-wrap items-center gap-0.5",
        "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)]",
        CONTAINER_SIZES[size] ?? CONTAINER_SIZES.md,
        className
      )}
    >
      ${options.map(
        (option) => html`
          <button
            key=${option.value}
            type="button"
            aria-pressed=${value === option.value}
            onClick=${() => onChange?.(option.value)}
            className=${cn(
              "shrink-0 whitespace-nowrap font-medium leading-none transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--v2-accent)]/50",
              SEGMENT_SIZES[size] ?? SEGMENT_SIZES.md,
              value === option.value
                ? "bg-[var(--v2-surface)] text-[var(--v2-text-strong)] shadow-[0_1px_3px_rgba(0,0,0,0.14)]"
                : "text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
            )}
          >
            ${option.label}
          </button>
        `
      )}
    </div>
  `;
}
