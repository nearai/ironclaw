/**
 * Collapsible
 *
 * Show/hide disclosure region built on @radix-ui/react-collapsible.
 * Unstyled by design — the trigger is usually a Button/IconButton and the
 * content brings its own layout. Only a minimal width default is applied.
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

export function CollapsibleTrigger(
  props: ComponentProps<typeof CollapsiblePrimitive.Trigger>
) {
  return <CollapsiblePrimitive.Trigger {...props} />;
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
