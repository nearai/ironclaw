/**
 * Tabs
 *
 * Underline tab row for switching between views or filters of the same
 * collection. Geometry comes from the shared control tokens so a tab row
 * lines up with Button/Select in adjacent toolbars: the row is
 * --v2-control-h-md tall with a hairline baseline, and the active tab
 * carries a 2px accent underline.
 *
 * This is the desktop/tablet vocabulary for single-select filters; below
 * `sm` callers should swap to a SelectMenu dropdown instead of shrinking
 * or scrolling the tab row.
 *
 * Props
 *   tabs      [{ value, label, count? }]
 *   value     currently selected tab value
 *   onChange  (value) => void
 *   ariaLabel accessible name for the tab list
 *   bordered  draw the hairline baseline (default true); pass false when a
 *             parent toolbar owns the baseline so right-side controls can
 *             share the same rule
 *   className layout additions for the row
 */
import { cn } from "../utils/cn";

export function Tabs({
  tabs = [],
  value,
  onChange = (_value) => {},
  ariaLabel = undefined,
  bordered = true,
  className = "",
}) {
  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      className={cn(
        "flex max-w-full items-end gap-1",
        bordered && "border-b border-[var(--v2-panel-border)]",
        className
      )}
    >
      {tabs.map((tab) => {
        const selected = tab.value === value;
        return (
          <button
            key={tab.value}
            type="button"
            role="tab"
            aria-selected={selected ? "true" : "false"}
            onClick={() => onChange(tab.value)}
            className={cn(
              "-mb-px inline-flex h-[var(--v2-control-h-md)] shrink-0 items-center gap-1.5",
              "border-b-2 px-[var(--v2-control-px-sm)] text-[13px] font-medium",
              "transition-colors focus-visible:outline-none focus-visible:ring-2",
              "focus-visible:ring-inset focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_40%,transparent)]",
              selected
                ? "border-[var(--v2-accent)] text-[var(--v2-text-strong)]"
                : "border-transparent text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
            )}
          >
            {tab.label}
            {tab.count != null && (
              <span
                className={cn(
                  "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full px-1",
                  "font-mono text-[10px] tabular-nums leading-none",
                  selected
                    ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                    : "bg-[var(--v2-surface-muted)] text-[var(--v2-text-muted)]"
                )}
              >
                {tab.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
