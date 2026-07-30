/**
 * StatStrip / StatTile
 *
 * The summary strip that heads list pages: a Panel holding a responsive grid
 * of bordered stat tiles. Promoted from the automations summary strip — the
 * richest of the app's six hand-rolled strips — whose tiles double as filter
 * buttons.
 *
 * StatStrip props
 *   columns   2 | 3 | 4 — grid columns at lg (all strips are 2-up at sm)
 *   children  StatTiles
 *
 * StatTile props
 *   label / value / tone / badgeLabel / detail / valueClassName — see StatCard
 *   onSelect    optional — renders the tile as a filter <button>
 *   isActive    accent highlight + aria-pressed for the selected filter
 *   selectTitle tooltip for the interactive tile (pass a translated string)
 */
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Card } from "../components/card";
import type { BadgeTone } from "../components/badge";
import { StatCard } from "./stat-card";

/* ─── StatStrip ───────────────────────────────────────────────────── */

const COLUMN_CLASSES = {
  2: "sm:grid-cols-2",
  3: "sm:grid-cols-2 lg:grid-cols-3",
  4: "sm:grid-cols-2 lg:grid-cols-4",
};

type StatStripProps = {
  columns?: keyof typeof COLUMN_CLASSES;
  children?: ReactNode;
  className?: string;
};

export function StatStrip({ columns = 3, children, className = "" }: StatStripProps) {
  return (
    <Card padding="sm" className={cn("sm:p-5", className)}>
      <div className={cn("grid gap-4", COLUMN_CLASSES[columns] ?? COLUMN_CLASSES[3])}>
        {children}
      </div>
    </Card>
  );
}

/* ─── StatTile ────────────────────────────────────────────────────── */

const TILE_BASE =
  "rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4 text-left";

type StatTileProps = {
  label: ReactNode;
  value: ReactNode;
  tone?: BadgeTone;
  badgeLabel?: ReactNode;
  detail?: ReactNode;
  valueClassName?: string;
  onSelect?: () => void;
  isActive?: boolean;
  selectTitle?: string;
  className?: string;
};

export function StatTile({
  label,
  value,
  tone = "muted",
  badgeLabel,
  detail,
  valueClassName,
  onSelect,
  isActive = false,
  selectTitle,
  className = "",
}: StatTileProps) {
  const inner = (
    <StatCard
      label={label}
      value={value}
      tone={tone}
      badgeLabel={badgeLabel}
      detail={detail}
      valueClassName={valueClassName}
      showDivider={false}
      className="px-0 py-0"
    />
  );

  if (!onSelect) {
    return (<div className={cn(TILE_BASE, className)}>{inner}</div>);
  }

  return (
    <button
      type="button"
      aria-pressed={isActive}
      title={selectTitle}
      onClick={onSelect}
      className={cn(
        TILE_BASE,
        "transition-colors",
        "hover:border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))] hover:bg-[var(--v2-surface-muted)]",
        "focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",
        isActive && "border-[var(--v2-accent)]/60 bg-[var(--v2-accent-soft)]/30",
        className
      )}
    >
      {inner}
    </button>
  );
}
