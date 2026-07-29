/**
 * Slider
 *
 * Range input built on @radix-ui/react-slider. Supports single and multi
 * thumb values; track/range/thumb all pull from the v2 tokens. Give each
 * thumb an accessible name via aria-label / aria-labelledby.
 *
 * Usage
 *   <Slider defaultValue={[40]} max={100} step={1} aria-label="Volume" />
 */
import * as SliderPrimitive from "@radix-ui/react-slider";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export function Slider({
  className,
  defaultValue,
  value,
  ...props
}: ComponentProps<typeof SliderPrimitive.Root>) {
  const thumbCount = (value ?? defaultValue ?? [0]).length;
  return (
    <SliderPrimitive.Root
      defaultValue={defaultValue}
      value={value}
      className={cn(
        "relative flex w-full touch-none select-none items-center",
        "data-[orientation=vertical]:h-44 data-[orientation=vertical]:w-auto data-[orientation=vertical]:flex-col",
        "data-[disabled]:opacity-50",
        className
      )}
      {...props}
    >
      <SliderPrimitive.Track
        className={cn(
          "relative grow overflow-hidden rounded-full bg-[var(--v2-surface-muted)]",
          "data-[orientation=horizontal]:h-1.5 data-[orientation=horizontal]:w-full",
          "data-[orientation=vertical]:h-full data-[orientation=vertical]:w-1.5"
        )}
      >
        <SliderPrimitive.Range
          className="absolute bg-[var(--v2-accent)] data-[orientation=horizontal]:h-full data-[orientation=vertical]:w-full"
        />
      </SliderPrimitive.Track>
      {Array.from({ length: thumbCount }, (_item, index) => (
        <SliderPrimitive.Thumb
          key={index}
          className={cn(
            "block h-4 w-4 rounded-full border-2 transition-colors",
            "border-[var(--v2-accent)] bg-[var(--v2-canvas-strong)]",
            "hover:bg-[var(--v2-surface-soft)]",
            "focus-visible:outline-none focus-visible:ring-2",
            "focus-visible:ring-[var(--v2-focus-ring)]",
            "data-[disabled]:pointer-events-none"
          )}
        />
      ))}
    </SliderPrimitive.Root>
  );
}
