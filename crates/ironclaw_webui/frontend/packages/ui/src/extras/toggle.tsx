/**
 * Toggle
 *
 * Two-state pressed button built on @radix-ui/react-toggle. Pressed state
 * fills with the accent-soft token so it reads as "selected" without
 * competing with primary actions.
 *
 * Usage
 *   <Toggle pressed={bold} onPressedChange={setBold} aria-label="Bold">B</Toggle>
 */
import * as TogglePrimitive from "@radix-ui/react-toggle";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

const TOGGLE_SIZES = {
  sm: "h-8 min-w-8 px-2 text-ui-sm rounded-[8px]",
  md: "h-9 min-w-9 px-2.5 text-ui rounded-[10px]",
  lg: "h-10 min-w-10 px-3 text-ui rounded-[10px]",
};

export type ToggleSize = keyof typeof TOGGLE_SIZES;

/** Shared by Toggle and ToggleGroupItem so grouped items match standalone ones. */
export function toggleClasses(size: ToggleSize = "md", className = "") {
  return cn(
    "inline-flex select-none items-center justify-center gap-1.5 border font-medium transition-colors",
    "border-transparent bg-transparent text-[var(--v2-text-muted)]",
    "hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",
    "focus-visible:outline-none focus-visible:ring-2",
    "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
    "disabled:pointer-events-none disabled:opacity-50",
    "data-[state=on]:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]",
    "data-[state=on]:bg-[var(--v2-accent-soft)] data-[state=on]:text-[var(--v2-accent-text)]",
    TOGGLE_SIZES[size] ?? TOGGLE_SIZES.md,
    className
  );
}

type ToggleProps = ComponentProps<typeof TogglePrimitive.Root> & {
  size?: ToggleSize;
};

export function Toggle({ className, size = "md", ...props }: ToggleProps) {
  return (
    <TogglePrimitive.Root
      className={toggleClasses(size, className)}
      {...props}
    />
  );
}
