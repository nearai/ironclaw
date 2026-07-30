/**
 * DetailList / DetailRow
 *
 * Key-value rows for detail panels. Promoted from the admin user-detail
 * `DetailRow` (label left, value right, hairline separators) and rendered as
 * a semantic definition list like the chat approval card — the app's four
 * competing key-value shapes fold into this one.
 *
 * DetailRow layouts
 *   "row"     (default) dt left, dd right-aligned — profile/summary panels
 *   "stacked" mono-caps dt above the dd — meta grids (job overview cells)
 *
 * Rows draw a top hairline except the first, so lists read as one block.
 */
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";

type DetailListProps = {
  children?: ReactNode;
  className?: string;
};

export function DetailList({ children, className = "" }: DetailListProps) {
  return (<dl className={cn("m-0", className)}>{children}</dl>);
}

const LAYOUTS = {
  row: {
    row: "flex items-start justify-between gap-4 py-3",
    term: "text-xs text-[var(--v2-text-muted)]",
    value: "text-right text-sm text-[var(--v2-text-strong)]",
  },
  stacked: {
    row: "py-3",
    term: "font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]",
    value: "mt-1.5 text-sm text-[var(--v2-text-strong)]",
  },
};

type DetailRowProps = {
  term: ReactNode;
  children?: ReactNode;
  layout?: keyof typeof LAYOUTS;
  className?: string;
};

export function DetailRow({ term, children, layout = "row", className = "" }: DetailRowProps) {
  const classes = LAYOUTS[layout] ?? LAYOUTS.row;
  return (
    <div
      className={cn(
        "border-t border-[var(--v2-panel-border)] first:border-0 first:pt-0",
        classes.row,
        className
      )}
    >
      <dt className={classes.term}>{term}</dt>
      <dd className={cn("m-0 min-w-0", classes.value)}>{children}</dd>
    </div>
  );
}
