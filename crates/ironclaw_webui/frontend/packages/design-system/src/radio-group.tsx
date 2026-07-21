/**
 * RadioGroup — `@radix-ui/react-radio-group` + IronClaw tokens.
 */
import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

export function RadioGroup({
  className = "",
  ...props
}: ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Root>) {
  return (
    <RadioGroupPrimitive.Root
      className={cn("grid gap-2", className)}
      {...props}
    />
  );
}

export function RadioGroupItem({
  className = "",
  ...props
}: ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Item>) {
  return (
    <RadioGroupPrimitive.Item
      className={cn(
        "aspect-square h-4 w-4 rounded-full border border-[var(--v2-panel-border)]",
        "bg-[var(--v2-input-bg)] text-[var(--v2-accent)]",
        "transition-[border-color,box-shadow] duration-[var(--v2-duration-fast)]",
        "hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
        "data-[state=checked]:border-[var(--v2-accent)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className
      )}
      {...props}
    >
      <RadioGroupPrimitive.Indicator className="relative flex h-full w-full items-center justify-center">
        <span className="absolute h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
      </RadioGroupPrimitive.Indicator>
    </RadioGroupPrimitive.Item>
  );
}
