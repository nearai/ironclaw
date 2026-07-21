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
import { cva, type VariantProps } from "class-variance-authority";
import type { ComponentPropsWithoutRef, ElementType, ReactNode } from "react";
import { cn } from "./cn";
import { Spinner } from "./spinner";

/* ── Gradient assets (Tailwind can't express these) ────────────────── */
/* The brand-blue ramp lives in the --v2-btn-* tokens (app.css),
   backfilled from the nux / ironclaw.com landing button language. */

const PRIMARY_BG = "var(--v2-btn-primary-bg)";
const PRIMARY_HOVER_BG = "var(--v2-btn-primary-bg-hover)";

/* Compact control-density scale (see "Control density" in app.css):
   heights/paddings come from --v2-control-* tokens. sm 28 / md 32 / lg 36. */

const buttonVariants = cva(
  [
    "inline-flex items-center justify-center font-medium select-none",
    "transition-[background,border-color,color,box-shadow,scale]",
    "duration-[var(--v2-duration-fast)] ease-[var(--v2-ease-standard)]",
    "active:scale-[0.97] disabled:active:scale-100",
    "disabled:cursor-not-allowed disabled:opacity-50",
    "focus-visible:outline-none focus-visible:ring-2",
    "focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1",
    "focus-visible:ring-offset-[var(--v2-canvas)]",
  ],
  {
    variants: {
      variant: {
        // Primary uses an inline gradient; class string only covers shared chrome.
        primary: "relative overflow-hidden text-[var(--v2-on-accent)] group hover:shadow-[var(--v2-shadow-accent-hover)]",
        outline:
          "border-2 border-[var(--v2-btn-secondary-border)] bg-[var(--v2-btn-secondary-bg)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-accent)] hover:border-[var(--v2-accent)] hover:text-[var(--v2-on-accent)] active:bg-[var(--v2-accent-strong)]",
        secondary:
          "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
        ghost:
          "border border-transparent bg-transparent text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",
        danger:
          "border border-[color-mix(in_srgb,var(--v2-danger-text)_60%,transparent)] bg-transparent text-[var(--v2-danger-text)] hover:bg-[var(--v2-danger-soft)] active:bg-[var(--v2-danger-soft)]",
      },
      size: {
        sm: "h-[var(--v2-control-h-sm)] rounded-[var(--v2-radius-sm)] px-[var(--v2-control-px-sm)] text-[length:var(--v2-font-size-caption)]",
        md: "h-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)] px-[var(--v2-control-px-md)] text-[length:var(--v2-font-size-body-sm)]",
        lg: "h-[var(--v2-control-h-lg)] rounded-[var(--v2-radius-md)] px-[var(--v2-control-px-lg)] text-[length:var(--v2-font-size-body)]",
        icon: "h-[var(--v2-control-h-md)] w-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)]",
        "icon-sm": "h-[var(--v2-control-h-sm)] w-[var(--v2-control-h-sm)] rounded-[var(--v2-radius-sm)]",
      },
      fullWidth: {
        true: "w-full",
        false: "",
      },
    },
    defaultVariants: {
      variant: "primary",
      size: "md",
      fullWidth: false,
    },
  }
);

type ButtonOwnProps = {
  children?: ReactNode;
  className?: string;
  fullWidth?: boolean;
  loading?: boolean;
  disabled?: boolean;
  as?: ElementType;
} & VariantProps<typeof buttonVariants>;

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
  const classes = cn(
    buttonVariants({ variant, size, fullWidth }),
    disabledAnchorClass,
    className
  );

  /* ── Primary: gradient + hover overlay ──────────────────────────── */
  if (variant === "primary") {
    return (
      <Element
        style={{
          background: PRIMARY_BG,
          border: "1px solid var(--v2-btn-primary-border)",
        }}
        className={classes}
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

  return (
    <Element
      className={classes}
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

export { buttonVariants };
