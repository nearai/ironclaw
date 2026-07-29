/**
 * Tabs
 *
 * Tabbed panels built on @radix-ui/react-tabs. The list renders as a soft
 * surface rail; the active trigger lifts onto the strong canvas token —
 * same treatment in light and dark since both come from the token set.
 *
 * Usage
 *   <Tabs defaultValue="overview">
 *     <TabsList>
 *       <TabsTrigger value="overview">Overview</TabsTrigger>
 *       <TabsTrigger value="logs">Logs</TabsTrigger>
 *     </TabsList>
 *     <TabsContent value="overview">…</TabsContent>
 *   </Tabs>
 */
import * as TabsPrimitive from "@radix-ui/react-tabs";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export const Tabs = TabsPrimitive.Root;

export function TabsList({
  className,
  ...props
}: ComponentProps<typeof TabsPrimitive.List>) {
  return (
    <TabsPrimitive.List
      className={cn(
        "inline-flex items-center gap-1 rounded-[10px] p-1",
        "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
        className
      )}
      {...props}
    />
  );
}

export function TabsTrigger({
  className,
  ...props
}: ComponentProps<typeof TabsPrimitive.Trigger>) {
  return (
    <TabsPrimitive.Trigger
      className={cn(
        "inline-flex items-center gap-1.5 rounded-[7px] px-3 py-1.5 text-ui font-medium",
        "text-[var(--v2-text-muted)] transition-colors",
        "hover:text-[var(--v2-text-strong)]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
        "disabled:pointer-events-none disabled:opacity-50",
        "data-[state=active]:bg-[var(--v2-canvas-strong)] data-[state=active]:text-[var(--v2-text-strong)]",
        "data-[state=active]:shadow-[0_1px_3px_rgba(0,0,0,0.12)]",
        className
      )}
      {...props}
    />
  );
}

export function TabsContent({
  className,
  ...props
}: ComponentProps<typeof TabsPrimitive.Content>) {
  return (
    <TabsPrimitive.Content
      className={cn(
        "mt-3 text-ui text-[var(--v2-text)]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
        className
      )}
      {...props}
    />
  );
}
