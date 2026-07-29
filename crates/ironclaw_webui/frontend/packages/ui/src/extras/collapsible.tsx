/**
 * Collapsible
 *
 * Show/hide disclosure region built on @radix-ui/react-collapsible.
 * Unstyled by design — the trigger is usually a Button/IconButton and the
 * content brings its own layout. Only a minimal width default plus the
 * shared focus-ring/disabled treatment on the trigger are applied.
 *
 * Usage
 *   <Collapsible open={open} onOpenChange={setOpen}>
 *     <CollapsibleTrigger asChild><Button variant="ghost">Toggle</Button></CollapsibleTrigger>
 *     <CollapsibleContent>Hidden details</CollapsibleContent>
 *   </Collapsible>
 */
import * as CollapsiblePrimitive from "@radix-ui/react-collapsible";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export function Collapsible({
  className,
  ...props
}: ComponentProps<typeof CollapsiblePrimitive.Root>) {
  return (
    <CollapsiblePrimitive.Root className={cn("w-full", className)} {...props} />
  );
}

export function CollapsibleTrigger({
  className,
  ...props
}: ComponentProps<typeof CollapsiblePrimitive.Trigger>) {
  return (
    <CollapsiblePrimitive.Trigger
      className={cn(
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className
      )}
      {...props}
    />
  );
}

export function CollapsibleContent({
  className,
  ...props
}: ComponentProps<typeof CollapsiblePrimitive.Content>) {
  return (
    <CollapsiblePrimitive.Content
      className={cn("overflow-hidden", className)}
      {...props}
    />
  );
}
