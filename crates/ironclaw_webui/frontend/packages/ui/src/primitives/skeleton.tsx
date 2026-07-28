/**
 * Skeleton
 *
 * Loading placeholder block. Replaces the `.v2-skeleton` CSS bridge class
 * with a component. Same visual: a static three-stop gradient over the
 * muted surface (the shimmer animation is disabled by the app's static
 * motion policy) with a 6px radius.
 *
 * Props
 *   className  sizing / layout classes (e.g. "h-8", "h-[460px]")
 *   ...rest    forwarded to the div (data-testid, …)
 */
import { cn } from "./cn";

const SKELETON_BG =
  "bg-[linear-gradient(90deg,var(--v2-surface-muted),color-mix(in_srgb,var(--v2-surface-muted)_64%,var(--v2-accent-soft)),var(--v2-surface-muted))]";

export function Skeleton({ className = "", ...rest }) {
  return (
    <div
      aria-hidden="true"
      className={cn("rounded-[6px]", SKELETON_BG, className)}
      {...rest}
    />
  );
}
