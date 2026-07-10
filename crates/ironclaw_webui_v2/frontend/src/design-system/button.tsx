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
 *   as        "button" | "a" | Link-like component (pass href/to via ...props)
 *   className string — for layout/spacing overrides (margin, width, etc.)
 *   children
 *   ...rest   forwarded to the element (type, onClick, href, …)
 */
import type { ComponentPropsWithoutRef, ElementType, ReactNode } from "react";
import { cn } from "../utils/cn";
import { Spinner } from "./spinner";

/* ── Gradient assets (Tailwind can't express these) ────────────────── */
/* The brand-blue ramp lives in the --v2-btn-* tokens (app.css),
   backfilled from the nux / ironclaw.com landing button language. */

const PRIMARY_BG = "var(--v2-btn-primary-bg)";
const PRIMARY_HOVER_BG = "var(--v2-btn-primary-bg-hover)";

/* ── Base ──────────────────────────────────────────────────────────── */

const BASE =
  "inline-flex items-center justify-center font-medium select-none " +
  // Press feedback: a subtle scale-down on :active so the control feels
  // like it registered the press (Emil Kowalski's 0.95–0.98 range). CSS
  // (not motion/react) on purpose — transitions are interruptible, run
  // off the main thread, and the app.css reduced-motion guard already
  // suppresses them wholesale. No scale on hover, no springs.
  // `scale` is in the transition list because Tailwind's scale-*
  // utilities set the standalone `scale` property, not `transform`.
  "transition-[background,border-color,color,box-shadow,scale] " +
  "duration-[var(--v2-duration-fast)] ease-[var(--v2-ease-standard)] " +
  "active:scale-[0.97] disabled:active:scale-100 " +
  "disabled:cursor-not-allowed disabled:opacity-50 " +
  "focus-visible:outline-none focus-visible:ring-2 " +
  "focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 " +
  "focus-visible:ring-offset-[var(--v2-canvas)]";

/* ── Size classes ──────────────────────────────────────────────────── */
/* Compact control-density scale (see "Control density" in app.css +
   DESIGN_SYSTEM.md §4): heights/paddings come from the
   --v2-control-* tokens so buttons, inputs, and future controls
   align in mixed rows. sm 28px / md 32px / lg 36px. */

const SIZES = {
  sm:
    "h-[var(--v2-control-h-sm)] rounded-[var(--v2-radius-sm)] " +
    "px-[var(--v2-control-px-sm)] text-[length:var(--v2-font-size-caption)]",
  md:
    "h-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)] " +
    "px-[var(--v2-control-px-md)] text-[length:var(--v2-font-size-body-sm)]",
  lg:
    "h-[var(--v2-control-h-lg)] rounded-[var(--v2-radius-md)] " +
    "px-[var(--v2-control-px-lg)] text-[length:var(--v2-font-size-body)]",
  icon:
    "h-[var(--v2-control-h-md)] w-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)]",
  "icon-sm":
    "h-[var(--v2-control-h-sm)] w-[var(--v2-control-h-sm)] rounded-[var(--v2-radius-sm)]",
};

/* ── Variant classes ───────────────────────────────────────────────── */
// Primary has no Tailwind variant string — it uses inline style for the gradient.
// Outline is the nux SECONDARY role: translucent accent outline that
// fills solid blue on hover.

const VARIANTS = {
  // hover text uses --v2-on-accent, NOT the `text-white` utility: the
  // legacy Tailwind-alias shim in app.css remaps `.text-white` /
  // `hover:text-white` to --v2-text-strong (dark ink in light mode).
  outline:
    "border-2 border-[var(--v2-btn-secondary-border)] bg-[var(--v2-btn-secondary-bg)] text-[var(--v2-text-strong)] " +
    "hover:bg-[var(--v2-accent)] hover:border-[var(--v2-accent)] hover:text-[var(--v2-on-accent)] " +
    "active:bg-[var(--v2-accent-strong)]",

  secondary:
    "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] " +
    "hover:bg-[var(--v2-surface-muted)] " +
    "hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",

  ghost:
    "border border-transparent bg-transparent text-[var(--v2-text-muted)] " +
    "hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",

  danger:
    "border border-[color-mix(in_srgb,var(--v2-danger-text)_60%,transparent)] bg-transparent text-[var(--v2-danger-text)] " +
    "hover:bg-[var(--v2-danger-soft)] active:bg-[var(--v2-danger-soft)]",
};

type ButtonOwnProps = {
  children?: ReactNode;
  className?: string;
  variant?: "primary" | keyof typeof VARIANTS;
  size?: keyof typeof SIZES;
  fullWidth?: boolean;
  loading?: boolean;
  disabled?: boolean;
  as?: ElementType;
};

type ButtonNativeProps = Omit<
  ComponentPropsWithoutRef<"button">,
  keyof ButtonOwnProps | "disabled"
>;

type LinkLikeProps = {
  href?: ComponentPropsWithoutRef<"a">["href"];
  target?: ComponentPropsWithoutRef<"a">["target"];
  rel?: ComponentPropsWithoutRef<"a">["rel"];
  download?: ComponentPropsWithoutRef<"a">["download"];
  to?: string;
  replace?: boolean;
  reloadDocument?: boolean;
  preventScrollReset?: boolean;
  relative?: "route" | "path";
  state?: unknown;
  viewTransition?: boolean;
};

type ButtonProps = ButtonOwnProps & ButtonNativeProps & LinkLikeProps;

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
}: ButtonProps) {
  const Element = Tag;
  const sizeClass  = SIZES[size] ?? SIZES.md;
  const fullClass  = fullWidth ? "w-full" : "";
  const isDisabled = disabled || loading;
  const isLinkLike = Tag === "a" || rest.href != null || rest.to != null;
  const disabledAnchorClass = isLinkLike && isDisabled ? "cursor-not-allowed opacity-50" : "";
  const nativeDisabled = isLinkLike ? undefined : isDisabled;
  const elementProps =
    isLinkLike && isDisabled
      ? {
          ...rest,
          onClick: (event: { preventDefault?: () => void; stopPropagation?: () => void }) => {
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
          border: "1px solid var(--v2-btn-primary-border)",
        }}
        className={cn(
          BASE,
          sizeClass,
          fullClass,
          disabledAnchorClass,
          // text-[var(--v2-on-accent)], not `text-white`: the legacy
          // alias shim in app.css remaps `.text-white` to the theme
          // ink color, which rendered dark text on the blue gradient
          // in light mode.
          "relative overflow-hidden text-[var(--v2-on-accent)] group",
          "hover:shadow-[var(--v2-shadow-accent-hover)]",
          className
        )}
        disabled={nativeDisabled}
        aria-disabled={isLinkLike && isDisabled ? true : undefined}
        aria-busy={loading || undefined}
        tabIndex={isLinkLike && isDisabled ? -1 : undefined}
        {...elementProps}
      >
        <span
          aria-hidden="true"
          style={{ background: PRIMARY_HOVER_BG }}
          className={
            "pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100 " +
            "transition-opacity duration-[var(--v2-duration-base)] ease-[var(--v2-ease-standard)]"
          }
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
      disabled={nativeDisabled}
      aria-disabled={isLinkLike && isDisabled ? true : undefined}
      aria-busy={loading || undefined}
      tabIndex={isLinkLike && isDisabled ? -1 : undefined}
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
