/**
 * VerticalTabs / VerticalTabsMobile
 *
 * The icon-tile section navigation used by settings, extensions and admin.
 * Those three pages carried character-for-character copies of the desktop
 * tile list and three different mobile treatments; this merges the best of
 * each — extensions' count badges, settings' mobile <details> disclosure —
 * onto token colors (the copies referenced a `v2-nav-active` class that no
 * longer exists, so their active state had quietly degraded to plain text).
 *
 * Props (shared)
 *   items     [{ id, label, icon?, count? }] — label pre-translated
 *   activeId  selected item id
 *   onSelect  (id) => void
 *   label     nav aria-label (pass a translated string)
 *
 * Selection is app-routed (URL-as-state), so items are buttons driven by the
 * caller, mirroring Breadcrumb's callback contract.
 */
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Icon, type IconName } from "../icons/icon";

export type VerticalTabItem = {
  id: string;
  label: ReactNode;
  icon?: IconName;
  count?: number;
};

type VerticalTabsProps = {
  items: VerticalTabItem[];
  activeId: string;
  onSelect: (id: string) => void;
  label: string;
  className?: string;
};

/* ─── Shared pieces ───────────────────────────────────────────────── */

function TabIconChip({ icon, active }: { icon?: IconName; active: boolean }) {
  if (!icon) return null;
  return (
    <span
      className={cn(
        "grid h-7 w-7 shrink-0 place-items-center rounded-md border transition-colors",
        active
          ? "border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
          : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] " +
            "group-hover:border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))] group-hover:text-[var(--v2-accent-text)]"
      )}
    >
      <Icon name={icon} className="h-3.5 w-3.5" />
    </span>
  );
}

function TabCount({ count }: { count?: number }) {
  if (count === undefined) return null;
  return (
    <span className="ml-auto shrink-0 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2 py-0.5 font-mono text-[0.625rem] text-[var(--v2-text-muted)]">
      {count}
    </span>
  );
}

/* ─── Desktop rail ────────────────────────────────────────────────── */

export function VerticalTabs({
  items,
  activeId,
  onSelect,
  label,
  className = "",
}: VerticalTabsProps) {
  return (
    <nav aria-label={label} className={cn("flex flex-col gap-1", className)}>
      {items.map((item) => {
        const active = item.id === activeId;
        return (
          <button
            key={item.id}
            type="button"
            aria-current={active ? "true" : undefined}
            onClick={() => onSelect(item.id)}
            className={cn(
              "group flex items-center gap-3 rounded-md px-3 py-2.5 text-left text-sm transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]",
              active
                ? "bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]"
            )}
          >
            <TabIconChip icon={item.icon} active={active} />
            <span className="min-w-0 truncate">{item.label}</span>
            <TabCount count={item.count} />
          </button>
        );
      })}
    </nav>
  );
}

/* ─── Mobile disclosure ───────────────────────────────────────────── */

export function VerticalTabsMobile({
  items,
  activeId,
  onSelect,
  label,
  className = "",
}: VerticalTabsProps) {
  const active = items.find((item) => item.id === activeId) || items[0];
  return (
    <details className={cn("group", className)}>
      <summary
        aria-label={label}
        className="flex cursor-pointer list-none items-center justify-between gap-3 rounded-[14px]
          border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-sm
          text-[var(--v2-text-strong)] [&::-webkit-details-marker]:hidden
          focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
      >
        <span className="flex min-w-0 items-center gap-2">
          {active?.icon &&
            (<Icon name={active.icon} className="h-4 w-4 shrink-0 text-[var(--v2-accent-text)]" />)}
          <span className="min-w-0 truncate">{active?.label}</span>
        </span>
        <Icon
          name="chevron"
          aria-hidden="true"
          className="h-4 w-4 shrink-0 text-[var(--v2-text-faint)] group-open:rotate-180"
        />
      </summary>
      <div className="mt-2 grid gap-1 rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-1">
        {items.map((item) => {
          const isActive = item.id === activeId;
          return (
            <button
              key={item.id}
              type="button"
              aria-current={isActive ? "true" : undefined}
              onClick={() => onSelect(item.id)}
              className={cn(
                "flex w-full items-center gap-3 rounded-[12px] px-3 py-2 text-left text-sm transition-colors",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]",
                isActive
                  ? "bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                  : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
              )}
            >
              <TabIconChip icon={item.icon} active={isActive} />
              <span className="min-w-0 truncate">{item.label}</span>
              <TabCount count={item.count} />
            </button>
          );
        })}
      </div>
    </details>
  );
}
