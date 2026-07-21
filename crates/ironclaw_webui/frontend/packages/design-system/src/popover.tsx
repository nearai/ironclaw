/**
 * Popover — `@radix-ui/react-popover` with IronClaw elevation.
 */
import * as PopoverPrimitive from "@radix-ui/react-popover";
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverAnchor = PopoverPrimitive.Anchor;
export const PopoverClose = PopoverPrimitive.Close;

export function PopoverContent({
  className = "",
  align = "center",
  sideOffset = 6,
  ...props
}: ComponentPropsWithoutRef<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        align={align}
        sideOffset={sideOffset}
        className={cn(
          "z-30 w-72 overflow-hidden rounded-[12px] p-4",
          "border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)]",
          "shadow-[var(--v2-shadow-menu)] outline-none",
          className
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}
