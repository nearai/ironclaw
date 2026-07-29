/**
 * Progress
 *
 * Determinate progress bar built on @radix-ui/react-progress. The track uses
 * the soft surface token, the indicator the accent token; width animates with
 * a plain transition so value changes glide instead of jumping.
 *
 * Usage
 *   <Progress value={64} aria-label="Upload progress" />
 */
import * as ProgressPrimitive from "@radix-ui/react-progress";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

type ProgressProps = ComponentProps<typeof ProgressPrimitive.Root> & {
  /** Indicator tone; defaults to accent. */
  tone?: "accent" | "positive" | "warning" | "danger";
};

const TONE_CLASSES = {
  accent: "bg-[var(--v2-accent)]",
  positive: "bg-[var(--v2-positive-text)]",
  warning: "bg-[var(--v2-warning-text)]",
  danger: "bg-[var(--v2-danger-text)]",
};

export function Progress({
  className,
  value,
  tone = "accent",
  ...props
}: ProgressProps) {
  const clamped = Math.min(100, Math.max(0, value ?? 0));
  return (
    <ProgressPrimitive.Root
      value={value}
      className={cn(
        "relative h-2 w-full overflow-hidden rounded-full",
        "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
        className
      )}
      {...props}
    >
      <ProgressPrimitive.Indicator
        className={cn(
          "h-full rounded-full transition-[width] duration-300",
          TONE_CLASSES[tone] ?? TONE_CLASSES.accent
        )}
        style={{ width: `${clamped}%` }}
      />
    </ProgressPrimitive.Root>
  );
}
