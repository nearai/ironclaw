/**
 * Button
 *
 * Single component — all visual styling via Tailwind + inline style for the
 * one thing Tailwind can't do (radial-gradient on primary).  No app.css
 * classes referenced.
 *
 * Props
 *   variant   "primary" | "outline" | "secondary" | "ghost" | "danger"
 *   size      "sm" | "md" (default) | "lg" | "icon" | "icon-sm"
 *   fullWidth boolean
 *   loading   boolean — shows an inline spinner, disables the button, sets
 *             aria-busy. The label stays visible so the button keeps its width.
 *   disabled  boolean
 *   as        "button" | "a" (renders anchor; pass href via ...props)
 *   className string — for layout/spacing overrides (margin, width, etc.)
 *   children
 *   ...rest   forwarded to the element (type, onClick, href, …)
 */
import { cn } from "../utils/cn";
import { Spinner } from "./spinner";

/* ── Gradient assets (Tailwind can't express these) ────────────────── */

const PRIMARY_BG =
  "radial-gradient(ellipse 100% 100% at 50% 130%, #4CA7E6 0%, #2882c8 65%)";
const PRIMARY_HOVER_BG =
  "radial-gradient(ellipse 200% 220% at 50% 110%, #5BBAF5 0%, #2882c8 60%)";

/* ── Base ──────────────────────────────────────────────────────────── */

const BASE =
  "inline-flex items-center justify-center font-semibold select-none " +
  "disabled:cursor-not-allowed disabled:opacity-50 " +
  "focus-visible:outline-none focus-visible:ring-2 " +
  "focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 " +
  "focus-visible:ring-offset-[var(--v2-canvas)]";

/* ── Size classes ──────────────────────────────────────────────────── */
/* Compact control-density scale (design system PR #5563): heights and
   paddings come from the --v2-control-* tokens (app.css) so buttons,
   inputs, selects, and tab rows align in mixed toolbar rows, and every
   corner comes from the shared --v2-radius-* ramp — no per-size magic
   radii. sm 28px / md 32px / lg 36px. */

const SIZES = {
  sm:
    "h-[var(--v2-control-h-sm)] rounded-[var(--v2-radius-sm)] " +
    "px-[var(--v2-control-px-sm)] text-xs",
  md:
    "h-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)] " +
    "px-[var(--v2-control-px-md)] text-[13px]",
  lg:
    "h-[var(--v2-control-h-lg)] rounded-[var(--v2-radius-md)] " +
    "px-[var(--v2-control-px-lg)] text-sm",
  icon:
    "h-[var(--v2-control-h-md)] w-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)]",
  "icon-sm":
    "h-[var(--v2-control-h-sm)] w-[var(--v2-control-h-sm)] rounded-[var(--v2-radius-sm)]",
};

/* ── Variant classes ───────────────────────────────────────────────── */
// Primary has no Tailwind variant string — it uses inline style for the gradient.

const VARIANTS = {
  outline:
    "border border-[rgba(76,167,230,0.7)] bg-transparent text-[#8fc8f2] " +
    "hover:bg-[rgba(76,167,230,0.1)] hover:border-[#4ca7e6] " +
    "active:bg-[rgba(76,167,230,0.15)]",

  secondary:
    "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] " +
    "hover:bg-[var(--v2-surface-muted)] " +
    "hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",

  ghost:
    "border border-transparent bg-transparent text-[var(--v2-text-muted)] " +
    "hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",

  danger:
    "border border-[rgba(217,101,116,0.6)] bg-transparent text-[#ff6480] " +
    "hover:bg-[rgba(217,101,116,0.08)] active:bg-[rgba(217,101,116,0.14)]",
};

/* ── Component ─────────────────────────────────────────────────────── */

export function Button({
  children,
  className = "",
  variant = "primary",
  size = "md",
  fullWidth = false,
  loading = false,
  disabled = false,
  as: Tag = "button",
  ...rest
}) {
  const Element: any = Tag;
  const sizeClass  = SIZES[size] ?? SIZES.md;
  const fullClass  = fullWidth ? "w-full" : "";
  const isDisabled = disabled || loading;
  const isAnchor = Tag === "a";
  const disabledAnchorClass = isAnchor && isDisabled ? "cursor-not-allowed opacity-50" : "";
  const elementProps =
    isAnchor && isDisabled
      ? {
          ...rest,
          onClick: (event) => {
            event?.preventDefault?.();
            event?.stopPropagation?.();
          },
        }
      : rest;

  /* ── Primary: gradient + hover overlay ──────────────────────────── */
  if (variant === "primary") {
    return (
      <Element
        style={{
          background: PRIMARY_BG,
          border: "1px solid rgba(76, 167, 230, 0.72)",
        }}
        className={cn(
          BASE,
          sizeClass,
          fullClass,
          disabledAnchorClass,
          "relative overflow-hidden text-white group",
          "hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",
          className
        )}
        disabled={isDisabled}
        aria-disabled={isAnchor && isDisabled ? true : undefined}
        aria-busy={loading || undefined}
        tabIndex={isAnchor && isDisabled ? -1 : undefined}
        {...elementProps}
      >
        <span
          aria-hidden="true"
          style={{ background: PRIMARY_HOVER_BG }}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
        />
        <span className="relative z-10 flex items-center gap-2">
          {loading && <Spinner />}
          {children}
        </span>
      </Element>
    );
  }

  /* ── All other variants ──────────────────────────────────────────── */
  const variantClass = VARIANTS[variant] ?? VARIANTS.outline;

  return (
    <Element
      className={cn(BASE, sizeClass, fullClass, disabledAnchorClass, variantClass, className)}
      disabled={isDisabled}
      aria-disabled={isAnchor && isDisabled ? true : undefined}
      aria-busy={loading || undefined}
      tabIndex={isAnchor && isDisabled ? -1 : undefined}
      {...elementProps}
    >
      {loading ? (
        <span className="inline-flex items-center gap-2">
          <Spinner />
          {children}
        </span>
      ) : children}
    </Element>
  );
}
