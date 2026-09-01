/**
 * Button
 *
 * Single component — all visual styling via Tailwind utilities backed by the
 * `--v2-*` design tokens, plus an inline style for the one thing Tailwind
 * can't express (the radial-gradient on primary), which itself reads a token.
 * No app.css component classes referenced.
 *
 * Shape and accent surface come from tokens, not literals: this component is
 * the reference for what "migrated" means (see `design-system/README.md` →
 * Design tokens). Retheming a button is an `app.css` edit, never an edit here.
 *
 * Props
 *   variant   "primary" | "tonal" | "outline" | "secondary" | "ghost" | "danger"
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

/* ── Gradient assets (Tailwind can't express these) ──────────────────
   The gradients themselves live in `app.css` as `--v2-accent-gradient*`;
   these constants only name the `var()` reference. Keep it that way — a
   literal here is a value the reskin cannot reach. */

const PRIMARY_BG = "var(--v2-accent-gradient)";
const PRIMARY_HOVER_BG = "var(--v2-accent-gradient-hover)";

/* ── Base ──────────────────────────────────────────────────────────── */

const BASE =
  "inline-flex items-center justify-center font-semibold select-none " +
  "disabled:cursor-not-allowed disabled:opacity-50 " +
  "focus-visible:outline-none focus-visible:ring-2 " +
  "focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 " +
  "focus-visible:ring-offset-[var(--v2-canvas)]";

/* ── Size classes ──────────────────────────────────────────────────── */

// Radii come from the control scale (`--v2-radius-control*`), so the shape of
// every button in the app moves from one place in app.css. The heights stay
// literal for now: they are a density decision, and density belongs to
// Phase 3a (#7781 WS3) along with the type ramp it has to move with.
const SIZES = {
  sm:      "h-9 rounded-control-sm px-3 text-ui-sm",
  md:      "min-h-[44px] rounded-control px-3.5 text-ui md:min-h-[50px] md:rounded-control-lg md:px-4",
  lg:      "min-h-[54px] rounded-control-xl px-6 text-ui-lg",
  icon:    "h-[44px] w-[44px] rounded-control md:h-[50px] md:w-[50px] md:rounded-control-lg",
  "icon-sm": "h-9 w-9 rounded-control-sm",
};

/* ── Variant classes ───────────────────────────────────────────────── */
// Primary has no Tailwind variant string — it uses inline style for the gradient.

const VARIANTS = {
  /* Medium emphasis: M3's primary-container. A tonal fill carrying a dark
     label, for accented actions that should not shout like `primary`. The
     label reads `--v2-accent-container-on` rather than a literal, because the
     container is light in one theme and dark in the other. */
  tonal:
    "border border-transparent bg-[var(--v2-accent-container)] text-[var(--v2-accent-container-on)] " +
    "hover:bg-[color-mix(in_srgb,var(--v2-accent-container)_88%,var(--v2-accent))]",

  outline:
    "border border-[color-mix(in_srgb,var(--v2-accent)_60%,var(--v2-panel-border))] " +
    "bg-transparent text-[var(--v2-accent-text)] " +
    "hover:bg-[var(--v2-accent-soft)] hover:border-[var(--v2-accent)] " +
    "active:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)]",

  secondary:
    "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] " +
    "hover:bg-[var(--v2-surface-muted)] " +
    "hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",

  ghost:
    "border border-transparent bg-transparent text-[var(--v2-text-muted)] " +
    "hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",

  danger:
    "border border-[color-mix(in_srgb,var(--v2-danger-text)_55%,var(--v2-panel-border))] " +
    "bg-transparent text-[var(--v2-danger-text)] " +
    "hover:bg-[var(--v2-danger-soft)] " +
    "active:bg-[color-mix(in_srgb,var(--v2-danger-text)_18%,transparent)]",
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
          border: "1px solid var(--v2-accent-edge)",
        }}
        className={cn(
          BASE,
          sizeClass,
          fullClass,
          disabledAnchorClass,
          "relative overflow-hidden text-[var(--v2-accent-on)] group",
          "hover:shadow-[var(--v2-accent-glow)]",
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
