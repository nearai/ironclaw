/**
 * Accordion
 *
 * Vertically stacked disclosure sections built on @radix-ui/react-accordion.
 * Single or multiple sections can be open (Radix `type` prop). Trigger rows
 * use the semantic type scale and rotate their chevron via the Radix
 * data-state attribute; borders and text pull from the v2 tokens.
 *
 * Usage
 *   <Accordion type="single" collapsible>
 *     <AccordionItem value="a">
 *       <AccordionTrigger>Section</AccordionTrigger>
 *       <AccordionContent>Body</AccordionContent>
 *     </AccordionItem>
 *   </Accordion>
 */
import * as AccordionPrimitive from "@radix-ui/react-accordion";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../primitives/icon";

export function Accordion({
  className,
  ...props
}: ComponentProps<typeof AccordionPrimitive.Root>) {
  return (
    <AccordionPrimitive.Root className={cn("w-full", className)} {...props} />
  );
}

export function AccordionItem({
  className,
  ...props
}: ComponentProps<typeof AccordionPrimitive.Item>) {
  return (
    <AccordionPrimitive.Item
      className={cn("border-b border-[var(--v2-panel-border)]", className)}
      {...props}
    />
  );
}

export function AccordionTrigger({
  className,
  children,
  ...props
}: ComponentProps<typeof AccordionPrimitive.Trigger>) {
  return (
    <AccordionPrimitive.Header className="flex">
      <AccordionPrimitive.Trigger
        className={cn(
          "group flex flex-1 items-center justify-between gap-3 py-3.5 text-left",
          "text-ui font-medium text-[var(--v2-text-strong)] transition-colors",
          "hover:text-[var(--v2-accent-text)]",
          "active:text-[var(--v2-accent)]",
          "focus-visible:outline-none focus-visible:ring-2",
          "focus-visible:ring-[var(--v2-focus-ring)]",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "disabled:hover:text-[var(--v2-text-strong)]",
          className
        )}
        {...props}
      >
        {children}
        <Icon
          name="chevron"
          className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)] transition-transform group-data-[state=open]:rotate-180"
        />
      </AccordionPrimitive.Trigger>
    </AccordionPrimitive.Header>
  );
}

export function AccordionContent({
  className,
  children,
  ...props
}: ComponentProps<typeof AccordionPrimitive.Content>) {
  return (
    <AccordionPrimitive.Content
      className="overflow-hidden text-ui text-[var(--v2-text-muted)]"
      {...props}
    >
      <div className={cn("pb-4 pt-0.5", className)}>{children}</div>
    </AccordionPrimitive.Content>
  );
}
