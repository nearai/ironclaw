/**
 * RadioGroup
 *
 * Exclusive-choice control built on @radix-ui/react-radio-group. Items render
 * as token-bordered circles with an accent dot when selected; pair each item
 * with a core Label via htmlFor/id.
 *
 * Usage
 *   <RadioGroup value={v} onValueChange={setV}>
 *     <div className="flex items-center gap-2">
 *       <RadioGroupItem value="a" id="opt-a" />
 *       <Label htmlFor="opt-a">Option A</Label>
 *     </div>
 *   </RadioGroup>
 */
import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export function RadioGroup({
  className,
  ...props
}: ComponentProps<typeof RadioGroupPrimitive.Root>) {
  return (
    <RadioGroupPrimitive.Root
      className={cn("grid gap-2.5", className)}
      {...props}
    />
  );
}

export function RadioGroupItem({
  className,
  ...props
}: ComponentProps<typeof RadioGroupPrimitive.Item>) {
  return (
    <RadioGroupPrimitive.Item
      className={cn(
        "grid h-[18px] w-[18px] shrink-0 place-items-center rounded-full border transition-colors",
        "border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)]",
        "hover:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        "data-[state=checked]:border-[var(--v2-accent)]",
        className
      )}
      {...props}
    >
      <RadioGroupPrimitive.Indicator className="grid place-items-center">
        <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
      </RadioGroupPrimitive.Indicator>
    </RadioGroupPrimitive.Item>
  );
}
