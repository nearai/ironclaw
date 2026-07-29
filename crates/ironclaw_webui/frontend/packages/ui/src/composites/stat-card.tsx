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
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Badge, type BadgeTone } from "../components/badge";

type StatCardProps = {
  label: ReactNode;
  value: ReactNode;
  tone?: BadgeTone;
  badgeLabel?: ReactNode;
  detail?: ReactNode;
  showDivider?: boolean;
  className?: string;
  valueClassName?: string;
};

export function StatCard({
  label,
  value,
  tone = "muted",
  badgeLabel = undefined,
  detail = "",
  showDivider = true,
  className = "",
  valueClassName = "text-[1.75rem] md:text-[2rem]",
}: StatCardProps) {
  return (
    <div
      className={cn(
        "px-1 py-4",
        showDivider && "border-t border-[var(--v2-panel-border)]",
        className
      )}
    >
      {/*
       * The Badge is `shrink-0` and a tone-name chip runs ~90px wide, which is
       * over half the width of a card in a six-across summary strip. Without
       * `flex-wrap` + a text basis, the text column collapses to ~40px: `detail`
       * wraps one word per line and an unbreakable label ("UNVERIFIED") spills
       * over the chip. Wrapping drops the chip onto its own line only when the
       * two genuinely cannot share one, so wider cards are unchanged.
       */}
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
        <div className="min-w-0 flex-1 basis-40">
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
