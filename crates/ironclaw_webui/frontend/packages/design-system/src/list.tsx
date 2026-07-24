/**
 * ListRow — the one row shape for tables, feeds, run steps, and pickers.
 *
 * Slot-based: leading (checkbox, icon chip, avatar), title, description,
 * meta (mono, right of description), trailing (badge, buttons). Passing
 * `onClick` makes the whole row an accessible button; otherwise it is a
 * static div. Rows divide themselves; the last row in a container drops
 * its divider automatically.
 *
 * Every list surface in the compositions is this component. If a list
 * needs a shape this can't express, extend it here rather than hand-
 * rolling a row.
 */
import type { ReactNode } from "react";
import { cn } from "./cn";

export interface ListRowProps {
  /** Left slot: checkbox, icon chip, avatar. */
  leading?: ReactNode;
  title: ReactNode;
  /** Secondary line under the title. */
  description?: ReactNode;
  /** Small mono line under the description (timestamps, ids, durations). */
  meta?: ReactNode;
  /** Right slot: badge, actions, mono values. */
  trailing?: ReactNode;
  /** Makes the row interactive (renders a button with hover state). */
  onClick?: () => void;
  /** Bottom divider; last row in a group drops it automatically. */
  divider?: boolean;
  /** Vertical density. */
  size?: "sm" | "md";
  /** Truncate the title to one line (tables). Turn off for feed receipts. */
  truncateTitle?: boolean;
  /** Cross-axis alignment; "start" tops-aligns slots for multi-line rows. */
  align?: "center" | "start";
  className?: string;
}

const PAD = { sm: "py-3", md: "py-3.5" };

export function ListRow({
  leading,
  title,
  description,
  meta,
  trailing,
  onClick,
  divider = true,
  size = "md",
  truncateTitle = true,
  align = "center",
  className = "",
}: ListRowProps) {
  const interactive = typeof onClick === "function";
  const Element = interactive ? "button" : "div";
  return (
    <Element
      {...(interactive ? { type: "button" as const, onClick } : {})}
      className={cn(
        "flex w-full gap-3 px-5 text-left",
        align === "start" ? "items-start" : "items-center",
        PAD[size] ?? PAD.md,
        divider && "border-b border-[var(--v2-panel-border)] last:border-b-0",
        interactive &&
          "transition-colors duration-[var(--v2-duration-fast)] hover:bg-[var(--v2-surface-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50",
        className
      )}
    >
      {leading && <span className="flex shrink-0 items-center">{leading}</span>}
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "block text-sm font-medium text-[var(--v2-text-strong)]",
            truncateTitle ? "truncate" : "leading-6"
          )}
        >
          {title}
        </span>
        {description && (
          <span className="mt-0.5 block text-xs leading-5 text-[var(--v2-text-muted)]">
            {description}
          </span>
        )}
        {meta && (
          <span className="mt-0.5 block font-mono text-xs text-[var(--v2-text-faint)]">{meta}</span>
        )}
      </span>
      {trailing && <span className="flex shrink-0 items-center gap-2">{trailing}</span>}
    </Element>
  );
}
