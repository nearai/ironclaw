/**
 * Switch
 *
 * On/off toggle built on @radix-ui/react-switch. Checked state fills the
 * track with the accent token; the thumb slides with a plain transform
 * transition. Pair with the core Label for captioned rows.
 *
 * Usage
 *   <Switch checked={enabled} onCheckedChange={setEnabled} aria-label="Notifications" />
 */
import * as SwitchPrimitive from "@radix-ui/react-switch";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export function Switch({
  className,
  ...props
}: ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      className={cn(
        "inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full border transition-colors",
        "border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)]",
        "data-[state=unchecked]:hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[var(--v2-focus-ring)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        "data-[state=checked]:border-[var(--v2-accent)] data-[state=checked]:bg-[var(--v2-accent)]",
        className
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        className={cn(
          "block h-4 w-4 translate-x-[3px] rounded-full bg-[var(--v2-canvas-strong)]",
          "shadow-[0_1px_2px_rgba(0,0,0,0.25)] transition-transform",
          "data-[state=checked]:translate-x-[17px] data-[state=checked]:bg-[var(--v2-inverse)]"
        )}
      />
    </SwitchPrimitive.Root>
  );
}
