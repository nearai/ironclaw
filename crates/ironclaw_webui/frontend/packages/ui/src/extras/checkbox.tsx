/**
 * Checkbox
 *
 * Tri-state checkbox built on @radix-ui/react-checkbox. Checked state fills
 * with the accent token; indeterminate renders a dash. Pairs with the core
 * Label / FormField for captioned rows.
 *
 * Usage
 *   <Checkbox checked={value} onCheckedChange={setValue} aria-label="Enable" />
 */
import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export function Checkbox({
  className,
  ...props
}: ComponentProps<typeof CheckboxPrimitive.Root>) {
  return (
    <CheckboxPrimitive.Root
      className={cn(
        "grid h-[18px] w-[18px] shrink-0 place-items-center rounded-[5px] border transition-colors",
        "border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)]",
        "hover:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        "data-[state=checked]:border-[var(--v2-accent)] data-[state=checked]:bg-[var(--v2-accent)]",
        "data-[state=indeterminate]:border-[var(--v2-accent)] data-[state=indeterminate]:bg-[var(--v2-accent)]",
        className
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator className="text-[var(--v2-inverse)]">
        <svg
          aria-hidden="true"
          viewBox="0 0 12 12"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          className="h-3 w-3"
        >
          {props.checked === "indeterminate" ? (
            <path d="M2.5 6h7" />
          ) : (
            <path d="m2.5 6.3 2.4 2.4 4.6-5.4" />
          )}
        </svg>
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}
