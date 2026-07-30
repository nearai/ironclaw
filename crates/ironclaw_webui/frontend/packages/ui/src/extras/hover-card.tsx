/**
 * HoverCard
 *
 * Rich preview surfaced on hover/focus, built on @radix-ui/react-hover-card.
 * For touch/click affordances use Popover instead — hover cards are a
 * pointer-only enhancement by design.
 *
 * Usage
 *   <HoverCard>
 *     <HoverCardTrigger asChild><a href="…">@ada</a></HoverCardTrigger>
 *     <HoverCardContent>Profile preview…</HoverCardContent>
 *   </HoverCard>
 */
import * as HoverCardPrimitive from "@radix-ui/react-hover-card";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";
import { OVERLAY_SURFACE_CLASSES } from "../primitives/overlay";

export const HoverCard = HoverCardPrimitive.Root;
export const HoverCardTrigger = HoverCardPrimitive.Trigger;

export function HoverCardContent({
  className,
  align = "center",
  sideOffset = 6,
  ...props
}: ComponentProps<typeof HoverCardPrimitive.Content>) {
  return (
    <HoverCardPrimitive.Portal>
      <HoverCardPrimitive.Content
        align={align}
        sideOffset={sideOffset}
        className={cn(
          OVERLAY_SURFACE_CLASSES,
          "w-64 p-4 text-ui text-[var(--v2-text)]",
          className
        )}
        {...props}
      />
    </HoverCardPrimitive.Portal>
  );
}
