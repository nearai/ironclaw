/**
 * Popover
 *
 * Click-triggered floating panel built on @radix-ui/react-popover.
 * Shares the overlay surface styling with the menus; content gets a roomier
 * padding default since popovers usually hold forms or prose, not items.
 *
 * Usage
 *   <Popover>
 *     <PopoverTrigger asChild><Button variant="secondary">Filters</Button></PopoverTrigger>
 *     <PopoverContent>…</PopoverContent>
 *   </Popover>
 */
import * as PopoverPrimitive from "@radix-ui/react-popover";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";
import { OVERLAY_SURFACE_CLASSES } from "./overlay";

export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverAnchor = PopoverPrimitive.Anchor;
export const PopoverClose = PopoverPrimitive.Close;

export function PopoverContent({
  className,
  align = "center",
  sideOffset = 6,
  ...props
}: ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        align={align}
        sideOffset={sideOffset}
        className={cn(
          OVERLAY_SURFACE_CLASSES,
          "w-72 p-4 text-ui text-[var(--v2-text)] outline-none",
          className
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}
