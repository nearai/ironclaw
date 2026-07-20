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

const SIZES = {
  sm:      "h-9 rounded-[10px] px-3 text-xs",
  md:      "min-h-[44px] rounded-[14px] px-3.5 text-[13px] md:min-h-[50px] md:rounded-[16px] md:px-4 md:text-sm",
  lg:      "min-h-[54px] rounded-[18px] px-6 text-base",
  icon:    "h-[44px] w-[44px] rounded-[14px] md:h-[50px] md:w-[50px] md:rounded-[16px]",
  "icon-sm": "h-9 w-9 rounded-[10px]",
};

/* ── Variant classes ───────────────────────────────────────────────── */
// Primary has no Tailwind variant string — it uses inline style for the gradient.

const VARIANTS = {
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
