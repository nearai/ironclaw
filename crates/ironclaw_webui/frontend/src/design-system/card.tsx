/**
 * Card / Panel
 *
 * Replaces the old .v2-panel CSS class with a proper React component.
 * All styling is via Tailwind arbitrary values backed by CSS variables so
 * light ↔ dark theme switching is automatic.
 *
 * Props
 *   variant   "default" | "bordered" | "subtle" | "inset"
 *   radius    "sm" | "md" (default) | "lg"
 *   padding   "none" (default) | "sm" | "md" | "lg"
 *   as        element tag, default "div"
 *   className string — layout / spacing additions
 *   children
 *
 * Sub-components (all optional, compose freely)
 *   <CardHeader>   — top section, optional bottom divider
 *   <CardBody>     — main content area
 *   <CardFooter>   — bottom section, optional top divider
 *   <CardLabel>    — mono-caps eyebrow label
 */
import { cn } from "../utils/cn";

/* ─── Variant ─────────────────────────────────────────────────────── */
// --v2-card-bg     : solid panel surface
// --v2-card-border : transparent in dark (shadow-only), subtle in light
// --v2-card-shadow : drop shadow in dark, none in light

const VARIANTS = {
  default:
    "bg-[var(--v2-card-bg)] border border-[var(--v2-card-border)] shadow-[var(--v2-card-shadow)]",
  bordered:
    "bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)] shadow-[var(--v2-card-shadow)]",
  subtle:
    "bg-[var(--v2-surface-soft)] border border-[var(--v2-panel-border)]",
  inset:
    "bg-[var(--v2-surface-muted)] border border-[var(--v2-panel-border)]",
};

/* ─── Radius ──────────────────────────────────────────────────────── */

const RADII = {
  sm: "rounded-[14px]",
  md: "rounded-[1.25rem] md:rounded-[1.5rem]",
  lg: "rounded-[1.5rem]",
};

/* ─── Padding ─────────────────────────────────────────────────────── */

const PADDINGS = {
  none: "",
  sm:   "p-4",
  md:   "p-5",
  lg:   "p-5 md:p-7",
};

/* ─── Card ────────────────────────────────────────────────────────── */

export function Card({
  children,
  className = "",
  variant = "default",
  radius = "md",
  padding = "none",
  as: Tag = "div",
  ...rest
}) {
  const Element: any = Tag;
  return (
    <Element
      className={cn(
        VARIANTS[variant] ?? VARIANTS.default,
        RADII[radius]    ?? RADII.md,
        PADDINGS[padding] ?? "",
        className
      )}
      {...rest}
    >
      {children}
    </Element>
  );
}

/* ─── CardHeader ──────────────────────────────────────────────────── */

export function CardHeader({ children, className = "", divider = false }) {
  return (
    <div
      className={cn(
        "px-5 py-4 md:px-7 md:py-5",
        divider && "border-b border-[var(--v2-panel-border)]",
        className
      )}
    >
      {children}
    </div>
  );
}

/* ─── CardBody ────────────────────────────────────────────────────── */

export function CardBody({ children, className = "" }) {
  return (
    <div className={cn("px-5 py-4 md:px-7 md:py-5", className)}>
      {children}
    </div>
  );
}

/* ─── CardFooter ──────────────────────────────────────────────────── */

export function CardFooter({ children, className = "", divider = true }) {
  return (
    <div
      className={cn(
        "px-5 py-4 md:px-7 md:py-5",
        divider && "border-t border-[var(--v2-panel-border)]",
        className
      )}
    >
      {children}
    </div>
  );
}

/* ─── CardLabel ───────────────────────────────────────────────────── */

/** Mono-caps eyebrow label — sits above section headings. */
export function CardLabel({ children, className = "" }) {
  return (
    <div
      className={cn(
        "font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.22em] text-[var(--v2-text-faint)]",
        className
      )}
    >
      {children}
    </div>
  );
}
