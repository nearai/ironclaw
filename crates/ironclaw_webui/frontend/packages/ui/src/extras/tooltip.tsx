/**
 * Tooltip
 *
 * Short label on hover/focus, built on @radix-ui/react-tooltip. Renders as a
 * compact inverse pill (strong-text background, inverse text) so it stands
 * apart from popovers. Wrap an app region in TooltipProvider once.
 *
 * Usage
 *   <TooltipProvider>
 *     <Tooltip>
 *       <TooltipTrigger asChild><IconButton icon="settings" label="Settings" /></TooltipTrigger>
 *       <TooltipContent>Settings</TooltipContent>
 *     </Tooltip>
 *   </TooltipProvider>
 */
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export const TooltipProvider = TooltipPrimitive.Provider;
export const Tooltip = TooltipPrimitive.Root;
export const TooltipTrigger = TooltipPrimitive.Trigger;

export function TooltipContent({
  className,
  sideOffset = 6,
  ...props
}: ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        sideOffset={sideOffset}
        className={cn(
          "z-50 max-w-64 rounded-[8px] px-2.5 py-1.5 text-ui-sm font-medium",
          "bg-[var(--v2-text-strong)] text-[var(--v2-inverse)]",
          "shadow-[0_10px_24px_-12px_rgba(0,0,0,0.5)]",
          className
        )}
        {...props}
      />
    </TooltipPrimitive.Portal>
  );
}
