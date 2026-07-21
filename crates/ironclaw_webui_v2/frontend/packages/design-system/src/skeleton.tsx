/**
 * Skeleton — loading placeholder using IronClaw surface tokens.
 */
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

export function Skeleton({
  className = "",
  ...props
}: ComponentPropsWithoutRef<"div">) {
  return (
    <div
      className={cn(
        "animate-pulse rounded-[var(--v2-radius-md)] bg-[var(--v2-surface-muted)]",
        className
      )}
      {...props}
    />
  );
}
