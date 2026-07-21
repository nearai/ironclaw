/**
 * Checkbox — `@radix-ui/react-checkbox` + IronClaw control tokens.
 */
import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import { Check } from "lucide-react";
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

type CheckboxProps = ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root> & {
  className?: string;
};

export function Checkbox({ className = "", ...props }: CheckboxProps) {
  return (
    <CheckboxPrimitive.Root
      className={cn(
        "peer grid h-4 w-4 shrink-0 place-items-center rounded-[4px] border",
        "border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)]",
        "transition-[background,border-color,box-shadow] duration-[var(--v2-duration-fast)]",
        "hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
        "data-[state=checked]:border-[var(--v2-accent)] data-[state=checked]:bg-[var(--v2-accent)]",
        "data-[state=checked]:text-[var(--v2-on-accent)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator className="grid place-items-center text-current">
        <Check className="h-3 w-3" strokeWidth={2.5} />
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}
