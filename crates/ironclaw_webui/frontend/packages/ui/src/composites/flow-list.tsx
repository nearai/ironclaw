/**
 * FlowList
 *
 * Numbered list of { title, description } items. Items may carry an `id`
 * for stable React reconciliation; without one the list position is used,
 * so duplicate titles are safe.
 */
import type { CSSProperties, ReactNode } from "react";

export type FlowListItem = {
  id?: string;
  title: ReactNode;
  description?: ReactNode;
};

export function FlowList({ items }: { items: FlowListItem[] }) {
  return (
    <div className="grid gap-3">
      {items.map(
        (item, index) => (
          <div
            key={item.id ?? index}
            className="grid grid-cols-[2.75rem_minmax(0,1fr)] gap-4 border-t border-[var(--v2-panel-border)] py-4"
            style={{ "--index": index } as CSSProperties}
          >
            <div className="font-mono text-xs text-[var(--v2-accent-text)]">
              {String(index + 1).padStart(2, "0")}
            </div>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                {item.title}
              </div>
              <div className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
                {item.description}
              </div>
            </div>
          </div>
        )
      )}
    </div>
  );
}
