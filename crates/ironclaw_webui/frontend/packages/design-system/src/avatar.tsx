/**
 * Avatar — `@radix-ui/react-avatar` + IronClaw surfaces.
 */
import * as AvatarPrimitive from "@radix-ui/react-avatar";
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

export function Avatar({
  className = "",
  ...props
}: ComponentPropsWithoutRef<typeof AvatarPrimitive.Root>) {
  return (
    <AvatarPrimitive.Root
      className={cn(
        "relative flex h-8 w-8 shrink-0 overflow-hidden rounded-full",
        "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
        className
      )}
      {...props}
    />
  );
}

export function AvatarImage({
  className = "",
  ...props
}: ComponentPropsWithoutRef<typeof AvatarPrimitive.Image>) {
  return (
    <AvatarPrimitive.Image
      className={cn("aspect-square h-full w-full object-cover", className)}
      {...props}
    />
  );
}

export function AvatarFallback({
  className = "",
  ...props
}: ComponentPropsWithoutRef<typeof AvatarPrimitive.Fallback>) {
  return (
    <AvatarPrimitive.Fallback
      className={cn(
        "flex h-full w-full items-center justify-center",
        "bg-[var(--v2-surface-muted)] font-mono text-[11px] font-medium",
        "text-[var(--v2-text-muted)]",
        className
      )}
      {...props}
    />
  );
}
