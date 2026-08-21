import type { HTMLAttributes } from "react";
import { cn } from "../utils/cn";

export function Skeleton({
  className = "",
  "aria-hidden": ariaHidden,
  "aria-label": ariaLabel,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("v2-skeleton", className)}
      aria-hidden={ariaHidden ?? (ariaLabel ? undefined : true)}
      aria-label={ariaLabel}
      {...rest}
    />
  );
}

export function SkeletonList({
  count = 3,
  className = "space-y-4",
  itemClassName = "",
  ...rest
}: HTMLAttributes<HTMLDivElement> & {
  count?: number;
  itemClassName?: string;
}) {
  return (
    <div className={className} {...rest}>
      {Array.from({ length: count }, (_, index) => (
        <Skeleton key={index} className={itemClassName} />
      ))}
    </div>
  );
}
