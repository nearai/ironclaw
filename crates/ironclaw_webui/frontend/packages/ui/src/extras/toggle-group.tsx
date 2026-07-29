/**
 * ToggleGroup
 *
 * Set of related toggles built on @radix-ui/react-toggle-group — single or
 * multiple selection via the Radix `type` prop. Items reuse the standalone
 * Toggle styling so grouped and lone toggles look identical.
 *
 * Usage
 *   <ToggleGroup type="single" value={align} onValueChange={setAlign}>
 *     <ToggleGroupItem value="left" aria-label="Align left">L</ToggleGroupItem>
 *     <ToggleGroupItem value="center" aria-label="Align center">C</ToggleGroupItem>
 *   </ToggleGroup>
 */
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";
import { toggleClasses, type ToggleSize } from "./toggle";

export function ToggleGroup({
  className,
  ...props
}: ComponentProps<typeof ToggleGroupPrimitive.Root>) {
  return (
    <ToggleGroupPrimitive.Root
      className={cn("inline-flex items-center gap-1", className)}
      {...props}
    />
  );
}

type ToggleGroupItemProps = ComponentProps<
  typeof ToggleGroupPrimitive.Item
> & {
  size?: ToggleSize;
};

export function ToggleGroupItem({
  className,
  size = "md",
  ...props
}: ToggleGroupItemProps) {
  return (
    <ToggleGroupPrimitive.Item
      className={toggleClasses(size, className)}
      {...props}
    />
  );
}
