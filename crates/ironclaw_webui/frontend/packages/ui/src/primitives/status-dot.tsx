/**
 * StatusDot
 *
 * The small round status indicator used in connection rows, list items and
 * provider cards. Consolidates the hand-rolled `h-2 w-2 rounded-full …`
 * spans. With no `tone` the dot renders `bg-current` and inherits its color
 * from the surrounding text — the pattern established by the chat
 * connection-status row.
 *
 * Props
 *   tone   "success" | "warning" | "danger" | "info" | "accent" | "muted"
 *          — omit to inherit the parent text color via bg-current
 *   size   "sm" (6px, default) | "md" (8px)
 *   pulse  boolean — breathing emphasis for live states (uses the sanctioned
 *          v2-breathe keyframe; suppressed under prefers-reduced-motion)
 */
import { cn } from "./cn";

const TONE_CLASSES = {
  success: "bg-[var(--v2-positive-text)]",
  warning: "bg-[var(--v2-warning-text)]",
  danger: "bg-[var(--v2-danger-text)]",
  info: "bg-[var(--v2-info-text)]",
  accent: "bg-[var(--v2-accent-text)]",
  muted: "bg-[var(--v2-text-faint)]",
};

const SIZE_CLASSES = {
  sm: "h-1.5 w-1.5",
  md: "h-2 w-2",
};

export type StatusDotTone = keyof typeof TONE_CLASSES;

type StatusDotProps = {
  tone?: StatusDotTone;
  size?: keyof typeof SIZE_CLASSES;
  pulse?: boolean;
  className?: string;
};

export function StatusDot({ tone, size = "sm", pulse = false, className = "" }: StatusDotProps) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block shrink-0 rounded-full",
        SIZE_CLASSES[size] ?? SIZE_CLASSES.sm,
        tone ? (TONE_CLASSES[tone] ?? TONE_CLASSES.muted) : "bg-current",
        pulse && "animate-[v2-breathe_2s_ease-in-out_infinite]",
        className
      )}
    />
  );
}
