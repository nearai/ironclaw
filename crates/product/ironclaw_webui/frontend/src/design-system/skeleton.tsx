import type { HTMLAttributes } from "react";
import { cn } from "../utils/cn";

export function Skeleton({
  className = "",
  "aria-hidden": ariaHidden,
  "aria-label": ariaLabel,
  role,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("v2-skeleton", className)}
      aria-hidden={ariaHidden ?? (ariaLabel ? undefined : true)}
      aria-label={ariaLabel}
      role={role ?? (ariaLabel ? "status" : undefined)}
      {...rest}
    />
  );
}

export function SkeletonList({
  "aria-label": ariaLabel,
  count = 3,
  className = "space-y-4",
  itemClassName = "",
  role,
  ...rest
}: HTMLAttributes<HTMLDivElement> & {
  count?: number;
  itemClassName?: string;
}) {
  return (
    <div
      className={className}
      aria-label={ariaLabel}
      role={role ?? (ariaLabel ? "status" : undefined)}
      {...rest}
    >
      {Array.from({ length: count }, (_, index) => (
        <Skeleton key={index} className={itemClassName} />
      ))}
    </div>
  );
}
