/**
 * FlowList
 *
 * Numbered list of { title, description } items.
 */
export function FlowList({ items }) {
  return (
    <div className="grid gap-3">
      {items.map(
        (item, index) => (
          <div
            key={item.title}
            className="grid grid-cols-[2.75rem_minmax(0,1fr)] gap-4 border-t border-[var(--v2-panel-border)] py-4"
            style={{ "--index": index } as any}
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
