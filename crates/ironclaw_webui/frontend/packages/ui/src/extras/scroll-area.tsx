/**
 * ScrollArea
 *
 * Custom-scrollbar container built on @radix-ui/react-scroll-area. The
 * scrollbar thumb uses a faint text-token mix so it reads in both themes
 * without stealing attention.
 *
 * Usage
 *   <ScrollArea className="h-64 rounded-[10px] border border-[var(--v2-panel-border)]">
 *     <div className="p-4">long content…</div>
 *   </ScrollArea>
 */
import * as ScrollAreaPrimitive from "@radix-ui/react-scroll-area";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export function ScrollArea({
  className,
  children,
  ...props
}: ComponentProps<typeof ScrollAreaPrimitive.Root>) {
  return (
    <ScrollAreaPrimitive.Root
      className={cn("relative overflow-hidden", className)}
      {...props}
    >
      <ScrollAreaPrimitive.Viewport className="h-full w-full rounded-[inherit]">
        {children}
      </ScrollAreaPrimitive.Viewport>
      <ScrollBar />
      <ScrollAreaPrimitive.Corner />
    </ScrollAreaPrimitive.Root>
  );
}

export function ScrollBar({
  className,
  orientation = "vertical",
  ...props
}: ComponentProps<typeof ScrollAreaPrimitive.Scrollbar>) {
  return (
    <ScrollAreaPrimitive.Scrollbar
      orientation={orientation}
      className={cn(
        "flex touch-none select-none p-0.5 transition-colors",
        orientation === "vertical" && "h-full w-2.5",
        orientation === "horizontal" && "h-2.5 w-full flex-col",
        className
      )}
      {...props}
    >
      <ScrollAreaPrimitive.Thumb
        className="relative flex-1 rounded-full bg-[color-mix(in_srgb,var(--v2-text-faint)_55%,transparent)]"
      />
    </ScrollAreaPrimitive.Scrollbar>
  );
}
