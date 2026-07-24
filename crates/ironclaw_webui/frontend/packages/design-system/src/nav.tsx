/**
 * NavList / NavItem — sidebar and rail navigation.
 *
 * NavItem is a single atomic control: icon, label, optional count, active
 * state. It renders a <button> by default and any link-like element via
 * `as` (anchor, router Link). Active items get `aria-current="page"`;
 * counts render in the mono meta face. NavList is the accessible wrapper.
 *
 * These exist so app shells never hand-roll nav rows: a generated or
 * hand-written sidebar is the same three props per destination.
 */
import type { ComponentPropsWithoutRef, ElementType, ReactNode } from "react";
import { cn } from "./cn";
import { Icon } from "./icons";

/* ── NavList ──────────────────────────────────────────────────────── */

export interface NavListProps extends ComponentPropsWithoutRef<"nav"> {
  /** Accessible name for the navigation region. */
  label: string;
  children?: ReactNode;
}

export function NavList({ label, className = "", children, ...rest }: NavListProps) {
  return (
    <nav aria-label={label} className={cn("grid gap-0.5", className)} {...rest}>
      {children}
    </nav>
  );
}

/* ── NavItem ──────────────────────────────────────────────────────── */

export interface NavItemProps {
  /** Icon name from the system set (see Components → Icon). */
  icon?: string;
  label: ReactNode;
  /** Right-aligned mono count or short meta (e.g. 8, "3 new"). */
  count?: ReactNode;
  /** Current destination. Sets aria-current="page". */
  active?: boolean;
  as?: ElementType;
  className?: string;
  [key: string]: unknown;
}

export function NavItem({
  icon,
  label,
  count,
  active = false,
  as: Tag = "button",
  className = "",
  ...rest
}: NavItemProps) {
  const Element = Tag as ElementType;
  const isButton = Tag === "button";
  return (
    <Element
      {...(isButton ? { type: "button" } : {})}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex w-full items-center gap-2 rounded-[var(--v2-radius-sm)] px-2 py-1.5 text-left text-sm",
        "transition-colors duration-[var(--v2-duration-fast)] ease-[var(--v2-ease-standard)]",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50",
        active
          ? "bg-[var(--v2-surface-muted)] font-medium text-[var(--v2-text-strong)]"
          : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
        className
      )}
      {...rest}
    >
      {icon && <Icon name={icon} className="h-4 w-4 shrink-0" />}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {count != null && (
        <span className="shrink-0 font-mono text-xs text-[var(--v2-text-faint)]">{count}</span>
      )}
    </Element>
  );
}
