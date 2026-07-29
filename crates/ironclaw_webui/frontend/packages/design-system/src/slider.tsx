/**
 * Slider — `@radix-ui/react-slider` + IronClaw accent track.
 */
import * as SliderPrimitive from "@radix-ui/react-slider";
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

export function Slider({
  className = "",
  ...props
}: ComponentPropsWithoutRef<typeof SliderPrimitive.Root>) {
  return (
    <SliderPrimitive.Root
      className={cn(
        "relative flex w-full touch-none items-center select-none",
        "disabled:pointer-events-none disabled:opacity-50",
        className
      )}
      {...props}
    >
      <SliderPrimitive.Track className="relative h-1.5 w-full grow overflow-hidden rounded-full bg-[var(--v2-surface-muted)]">
        <SliderPrimitive.Range className="absolute h-full bg-[var(--v2-accent)]" />
      </SliderPrimitive.Track>
      <SliderPrimitive.Thumb
        className={cn(
          "block h-4 w-4 rounded-full border border-[var(--v2-accent)] bg-[var(--v2-inverse)]",
          "shadow-sm transition-[box-shadow,scale] duration-[var(--v2-duration-fast)]",
          "focus-visible:outline-none focus-visible:ring-2",
          "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
          "active:scale-[0.97]"
        )}
      />
    </SliderPrimitive.Root>
  );
}
