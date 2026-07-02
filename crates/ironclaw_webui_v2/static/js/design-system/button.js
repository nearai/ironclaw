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
 *   as        "button" | "a" (renders anchor; pass href via ...props)
 *   className string — for layout/spacing overrides (margin, width, etc.)
 *   children
 *   ...rest   forwarded to the element (type, disabled, onClick, href, …)
 */
import { html } from "../lib/html.js";
import { cn } from "../utils/cn.js";

/* ── Gradient assets (Tailwind can't express these) ────────────────── */
/* The brand-blue ramp lives in the --v2-btn-* tokens (app.css),
   backfilled from the nux / ironclaw.com landing button language. */

const PRIMARY_BG = "var(--v2-btn-primary-bg)";
const PRIMARY_HOVER_BG = "var(--v2-btn-primary-bg-hover)";

/* ── Base ──────────────────────────────────────────────────────────── */

const BASE =
  "inline-flex items-center justify-center font-medium select-none " +
  "transition-[background,border-color,color,box-shadow] " +
  "duration-[var(--v2-duration-fast)] ease-[var(--v2-ease-standard)] " +
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

/* ── Component ─────────────────────────────────────────────────────── */

export function Button({
  children,
  className = "",
  variant = "primary",
  size = "md",
  fullWidth = false,
  as: Tag = "button",
  ...rest
}) {
  const sizeClass  = SIZES[size] ?? SIZES.md;
  const fullClass  = fullWidth ? "w-full" : "";

  /* ── Primary: gradient + hover overlay ──────────────────────────── */
  if (variant === "primary") {
    return html`
      <${Tag}
        style=${{
          background: PRIMARY_BG,
          border: "1px solid var(--v2-btn-primary-border)",
        }}
        className=${cn(
          BASE,
          sizeClass,
          fullClass,
          // text-[var(--v2-on-accent)], not `text-white`: the legacy
          // alias shim in app.css remaps `.text-white` to the theme
          // ink color, which rendered dark text on the blue gradient
          // in light mode.
          "relative overflow-hidden text-[var(--v2-on-accent)] group",
          "hover:shadow-[var(--v2-shadow-accent-hover)]",
          className
        )}
        ...${rest}
      >
        <span
          aria-hidden="true"
          style=${{ background: PRIMARY_HOVER_BG }}
          className=${
            "pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100 " +
            "transition-opacity duration-[var(--v2-duration-base)] ease-[var(--v2-ease-standard)]"
          }
        />
        <span className="relative z-10 flex items-center gap-2">
          ${children}
        </span>
      <//>
    `;
  }

  /* ── All other variants ──────────────────────────────────────────── */
  const variantClass = VARIANTS[variant] ?? VARIANTS.outline;

  return html`
    <${Tag}
      className=${cn(BASE, sizeClass, fullClass, variantClass, className)}
      ...${rest}
    >
      ${children}
    <//>
  `;
}
