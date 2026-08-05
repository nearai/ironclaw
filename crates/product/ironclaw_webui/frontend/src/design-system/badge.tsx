/**
 * Badge / StatusPill
 *
 * A small labelled chip with a coloured dot.  All styling is via Tailwind
 * arbitrary values — no app.css classes.
 *
 * Props
 *   tone     "success" | "warning" | "danger" | "muted" | "signal" | "info"
 *   label    string
 *   dot      boolean (default true)
 *   size     "sm" (default) | "md"
 *   className string
 */
import { cn } from "../utils/cn";

/* ── Tone maps ────────────────────────────────────────────────────────── */

const toneClasses = {
  success:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
  positive:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
  signal:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
  warning:
    "border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",
  copper:
    "border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",
  danger:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",
  info:
    "border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",
  accent:
    "border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",
  muted:
    "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]",
};

const sizeClasses = {
  sm: "h-6 gap-1.5 rounded-full px-2 text-[0.625rem] tracking-[0.12em]",
  md: "h-7 gap-2 rounded-full px-2.5 text-[0.6875rem] tracking-[0.12em]",
};

export function Badge({ tone = "muted", label, dot = true, size = "md", className = "" }) {
  const isLive = tone === "success" || tone === "positive" || tone === "signal";
  return (
    <span
      className={cn(
        // `whitespace-nowrap` + `shrink-0` keep the chip on one line: CJK and
        // other space-free scripts wrap between any two characters, so a
        // translated tone label like "信号" would otherwise stack vertically
        // inside the fixed-height pill.
        "inline-flex shrink-0 items-center whitespace-nowrap border font-mono uppercase",
        sizeClasses[size] ?? sizeClasses.md,
        toneClasses[tone] ?? toneClasses.muted,
        className
      )}
    >
      {dot &&
        (<span
          className={cn(
            "h-1.5 w-1.5 shrink-0 rounded-full bg-current",
            isLive && "animate-[v2-breathe_2s_ease-in-out_infinite]"
          )}
        />)}
      {label}
    </span>
  );
}

/**
 * Alias kept for backwards-compat with existing imports.
 * Prefer <Badge> in new code.
 */
export const StatusPill = Badge;
