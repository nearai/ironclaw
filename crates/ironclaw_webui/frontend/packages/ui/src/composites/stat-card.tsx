/**
 * StatCard
 *
 * A labelled metric card used in summary strips and admin dashboards.
 *
 * Props
 *   label      string
 *   value      string | number
 *   tone       Badge tone
 *   badgeLabel string (optional) — Badge text; defaults to the tone keyword.
 *     Pass a translated label so the chip is not an English tone name.
 *   detail     string (optional sub-text)
 *   showDivider boolean
 *   className  string
 *   valueClassName string (optional) — overrides the value font-size classes.
 *     Defaults to the large numeric size; pass a smaller size for text values
 *     (e.g. a date) that would otherwise truncate. Note: `cn()` only
 *     concatenates (no tailwind-merge), so this REPLACES the size classes
 *     rather than appending to them.
 */
import { cn } from "../primitives/cn";
import { Badge } from "../components/badge";

export function StatCard({
  label,
  value,
  tone = "muted",
  badgeLabel = undefined,
  detail = "",
  showDivider = true,
  className = "",
  valueClassName = "text-[1.75rem] md:text-[2rem]",
}) {
  return (
    <div
      className={cn(
        "px-1 py-4",
        showDivider && "border-t border-[var(--v2-panel-border)]",
        className
      )}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div
            className="font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
          >
            {label}
          </div>
          <div
            className={cn(
              "mt-3 truncate font-medium tracking-[-0.05em] text-[var(--v2-text-strong)]",
              valueClassName
            )}
          >
            {value}
          </div>
          {detail &&
          (<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            {detail}
          </div>)}
        </div>
        <Badge tone={tone} label={badgeLabel ?? tone} />
      </div>
    </div>
  );
}
