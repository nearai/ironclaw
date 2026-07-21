/**
 * Switch — `@radix-ui/react-switch` + IronClaw accent.
 */
import * as SwitchPrimitive from "@radix-ui/react-switch";
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

type SwitchProps = ComponentPropsWithoutRef<typeof SwitchPrimitive.Root> & {
  className?: string;
};

export function Switch({ className = "", ...props }: SwitchProps) {
  return (
    <SwitchPrimitive.Root
      className={cn(
        "peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border border-transparent",
        "bg-[var(--v2-surface-muted)] transition-colors duration-[var(--v2-duration-fast)]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
        "data-[state=checked]:bg-[var(--v2-accent)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        className={cn(
          "pointer-events-none block h-4 w-4 rounded-full bg-[var(--v2-inverse)] shadow-sm",
          "transition-transform duration-[var(--v2-duration-fast)] ease-[var(--v2-ease-standard)]",
          "data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0.5"
        )}
      />
    </SwitchPrimitive.Root>
  );
}
