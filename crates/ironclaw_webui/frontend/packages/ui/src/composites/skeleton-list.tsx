/**
 * SkeletonList
 *
 * A stack of Skeleton blocks standing in for a loading list. Replaces the
 * `[1, 2, 3].map(... <div className="v2-skeleton h-28" />)` arrangements
 * every page hand-rolled with its own heights and repeat counts.
 *
 * Props
 *   count          number of placeholder rows (default 3)
 *   itemClassName  size/shape of each row; REPLACES the default h-28
 *                  rounded-[18px] (cn is concat-only)
 *   label          optional accessible loading label — adds role="status"
 *                  (the blocks themselves stay aria-hidden)
 */
import { cn } from "../primitives/cn";
import { Skeleton } from "../primitives/skeleton";

type SkeletonListProps = {
  count?: number;
  itemClassName?: string;
  label?: string;
  className?: string;
};

export function SkeletonList({
  count = 3,
  itemClassName = "",
  label,
  className = "",
}: SkeletonListProps) {
  return (
    <div
      role={label ? "status" : undefined}
      aria-label={label}
      className={cn("space-y-4", className)}
    >
      {Array.from({ length: count }, (_, index) => (
        <Skeleton key={index} className={itemClassName || "h-28 rounded-[18px]"} />
      ))}
    </div>
  );
}
