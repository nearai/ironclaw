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
   paddings come from the --v2-control-* tokens so buttons, inputs, and
   toolbar controls align in mixed rows. sm 28px / md 32px / lg 36px. */
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
    "hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))] " +
    // Icons read as muted by default and lighten to match the label on hover.
    "[&_svg]:text-[var(--v2-text-muted)] hover:[&_svg]:text-[var(--v2-text-strong)] [&_svg]:transition-colors",

  ghost:
    "border border-transparent bg-transparent text-[var(--v2-text-muted)] " +
    "hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)] " +
    "[&_svg]:text-[var(--v2-text-muted)] hover:[&_svg]:text-[var(--v2-text-strong)] [&_svg]:transition-colors",

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
          border: "1px solid rgba(76, 167, 230, 0.72)",
        }}
        className=${cn(
          BASE,
          sizeClass,
          fullClass,
          // text-[var(--v2-on-accent)], not `text-white`: the legacy compat
          // shim in app.css remaps `.text-white` to the theme ink, which
          // would render dark text on the accent gradient in light mode.
          "relative overflow-hidden text-[var(--v2-on-accent)] group",
          "hover:shadow-[0_24px_24px_-20px_rgba(76,167,230,0.55)]",
          className
        )}
        ...${rest}
      >
        <span
          aria-hidden="true"
          style=${{ background: PRIMARY_HOVER_BG }}
          className="pointer-events-none absolute inset-0 opacity-0 group-hover:opacity-100"
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
      <span className="flex items-center justify-center">${children}</span>
    <//>
  `;
}
