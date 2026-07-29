/**
 * Breadcrumb
 *
 * Mono-spaced path breadcrumb (extracted from the workspace file browser).
 * Every crumb is a button so navigation stays URL-as-state driven by the
 * caller; the first item is the root and is never truncated.
 *
 * Props
 *   label     nav aria-label
 *   items     [{ key?, label, onSelect }] — root first
 *   className layout additions
 */
import React from "react";
import { cn } from "../primitives/cn";

export type BreadcrumbItem = {
  key?: string;
  label: string;
  onSelect: () => void;
};

export function Breadcrumb({
  label,
  items,
  className = "",
}: {
  label: string;
  items: BreadcrumbItem[];
  className?: string;
}) {
  return (
    <nav
      aria-label={label}
      className={cn("flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm", className)}
    >
      {items.map((item, index) => (
        <React.Fragment key={item.key ?? `${index}:${item.label}`}>
          {/* faint, not muted: the original used text-iron-400 (--v2-text-faint) */}
          {index > 0 && (<span className="text-[var(--v2-text-faint)]">/</span>)}
          <button
            type="button"
            onClick={item.onSelect}
            className={cn(
              "text-[var(--v2-accent-text)] hover:underline",
              index > 0 && "max-w-[220px] truncate"
            )}
          >
            {item.label}
          </button>
        </React.Fragment>
      ))}
    </nav>
  );
}
