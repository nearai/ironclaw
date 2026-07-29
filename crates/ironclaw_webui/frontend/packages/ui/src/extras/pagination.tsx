/**
 * Pagination
 *
 * Page navigation with correct landmark semantics: <nav aria-label> around a
 * list of page buttons, aria-current="page" on the active page. Two layers:
 *
 *   - Primitives: Pagination / PaginationContent / PaginationItem /
 *     PaginationButton / PaginationPrevious / PaginationNext /
 *     PaginationEllipsis — compose freely (e.g. router links via `as`-less
 *     children, since PaginationButton is a plain <button>).
 *   - Convenience: <SimplePagination page pageCount onPageChange /> renders a
 *     windowed page list with ellipses.
 */
import type { ComponentProps, ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../primitives/icon";

export function Pagination({
  className,
  ...props
}: ComponentProps<"nav">) {
  return (
    <nav
      aria-label={props["aria-label"] ?? "Pagination"}
      className={cn("mx-auto flex w-full justify-center", className)}
      {...props}
    />
  );
}

export function PaginationContent({
  className,
  ...props
}: ComponentProps<"ul">) {
  return (
    <ul className={cn("flex list-none items-center gap-1", className)} {...props} />
  );
}

export function PaginationItem(props: ComponentProps<"li">) {
  return <li {...props} />;
}

type PaginationButtonProps = ComponentProps<"button"> & {
  /** Marks the current page (aria-current="page" + accent styling). */
  isActive?: boolean;
};

export function PaginationButton({
  className,
  isActive = false,
  ...props
}: PaginationButtonProps) {
  return (
    <button
      type="button"
      aria-current={isActive ? "page" : undefined}
      className={cn(
        "inline-flex h-9 min-w-9 items-center justify-center gap-1 rounded-[10px] border px-2 text-ui font-medium transition-colors",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        isActive
          ? "border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
          : "border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",
        className
      )}
      {...props}
    />
  );
}

type PaginationEdgeProps = ComponentProps<"button"> & {
  label?: ReactNode;
};

export function PaginationPrevious({
  className,
  label = "Previous",
  ...props
}: PaginationEdgeProps) {
  return (
    <PaginationButton className={cn("px-2.5", className)} {...props}>
      <Icon name="chevron" className="h-3.5 w-3.5 rotate-90" />
      {label}
    </PaginationButton>
  );
}

export function PaginationNext({
  className,
  label = "Next",
  ...props
}: PaginationEdgeProps) {
  return (
    <PaginationButton className={cn("px-2.5", className)} {...props}>
      {label}
      <Icon name="chevron" className="h-3.5 w-3.5 -rotate-90" />
    </PaginationButton>
  );
}

export function PaginationEllipsis({
  className,
  ...props
}: ComponentProps<"span">) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "grid h-9 w-9 place-items-center text-ui text-[var(--v2-text-faint)]",
        className
      )}
      {...props}
    >
      …
    </span>
  );
}

/* ── Convenience wrapper ───────────────────────────────────────────── */

/** Windowed page numbers: 1 … (page-1) page (page+1) … last. */
function pageWindow(page: number, pageCount: number): (number | "gap")[] {
  if (pageCount <= 7) {
    return Array.from({ length: pageCount }, (_v, index) => index + 1);
  }
  const middle = [page - 1, page, page + 1].filter(
    (candidate) => candidate > 1 && candidate < pageCount
  );
  const items: (number | "gap")[] = [1];
  if ((middle[0] ?? pageCount) > 2) items.push("gap");
  items.push(...middle);
  if ((middle[middle.length - 1] ?? 0) < pageCount - 1) items.push("gap");
  items.push(pageCount);
  return items;
}

type SimplePaginationProps = {
  /** Current 1-based page. */
  page: number;
  /** Total number of pages (>= 1). */
  pageCount: number;
  onPageChange: (page: number) => void;
  /** nav aria-label; defaults to "Pagination". */
  label?: string;
  previousLabel?: ReactNode;
  nextLabel?: ReactNode;
  className?: string;
};

export function SimplePagination({
  page,
  pageCount,
  onPageChange,
  label = "Pagination",
  previousLabel,
  nextLabel,
  className,
}: SimplePaginationProps) {
  return (
    <Pagination aria-label={label} className={className}>
      <PaginationContent>
        <PaginationItem>
          <PaginationPrevious
            label={previousLabel}
            disabled={page <= 1}
            onClick={() => onPageChange(page - 1)}
          />
        </PaginationItem>
        {pageWindow(page, pageCount).map((entry, index) => (
          <PaginationItem key={entry === "gap" ? `gap-${index}` : entry}>
            {entry === "gap" ? (
              <PaginationEllipsis />
            ) : (
              <PaginationButton
                isActive={entry === page}
                onClick={() => onPageChange(entry)}
              >
                {entry}
              </PaginationButton>
            )}
          </PaginationItem>
        ))}
        <PaginationItem>
          <PaginationNext
            label={nextLabel}
            disabled={page >= pageCount}
            onClick={() => onPageChange(page + 1)}
          />
        </PaginationItem>
      </PaginationContent>
    </Pagination>
  );
}
