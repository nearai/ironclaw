/**
 * Tabs
 *
 * Underline tab row for switching between views or filters of the same
 * collection. Geometry comes from the shared control tokens so a tab row
 * lines up with Button/Select in adjacent toolbars: the row is
 * --v2-control-h-md tall with a hairline baseline, and the active tab
 * carries a 2px accent underline.
 *
 * The underline is a shared-layout element (motion/react layoutId): when
 * the selection moves, it slides between tabs instead of blinking. The
 * slide is a quick tween (MOTION_DURATION.menu + strong ease-out — no
 * spring, selection is a pointer/keyboard action, not a playful pop) and
 * collapses to an instant swap under prefers-reduced-motion. Focus
 * traversal (Tab/arrow keys) never animates — only selection changes do.
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
import React from "react";
import { motion } from "motion/react";
import { cn } from "../utils/cn";
import { MOTION_DURATION, MOTION_EASE_OUT, useReducedMotion } from "./motion";

export function Tabs({
  tabs = [],
  value,
  onChange = (_value) => {},
  ariaLabel = undefined,
  bordered = true,
  className = "",
}) {
  // Unique per Tabs instance so two tab rows on one page never trade
  // underlines through the shared-layout system.
  const underlineId = React.useId();
  const reducedMotion = useReducedMotion();

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      className={cn(
        "flex max-w-full items-stretch gap-1",
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
              // Taller than a plain control so the label keeps breathing room
              // above the underline; the row stretches further when a parent
              // toolbar is taller, keeping the label vertically centered
              // against adjacent controls while the underline stays on the
              // shared hairline.
              "relative -mb-px inline-flex min-h-[calc(var(--v2-control-h-md)+var(--v2-control-px-sm))] shrink-0 items-center gap-1.5",
              "border-b-2 border-transparent px-[var(--v2-control-px-sm)] text-[13px] font-medium",
              "transition-colors focus-visible:outline-none focus-visible:ring-2",
              "focus-visible:ring-inset focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_40%,transparent)]",
              selected
                ? "text-[var(--v2-text-strong)]"
                : "text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
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
            {selected && (
              <motion.span
                aria-hidden="true"
                layoutId={underlineId}
                // Sits on the same hairline the border-b-2 used to paint:
                // the button keeps a transparent 2px border for geometry,
                // and this element draws the accent line.
                className="absolute inset-x-0 -bottom-[2px] h-[2px] bg-[var(--v2-accent)]"
                transition={
                  reducedMotion
                    ? { duration: 0 }
                    : { duration: MOTION_DURATION.menu, ease: MOTION_EASE_OUT }
                }
              />
            )}
          </button>
        );
      })}
    </div>
  );
}
