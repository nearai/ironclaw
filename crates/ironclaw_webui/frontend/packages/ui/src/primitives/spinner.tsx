/**
 * Spinner
 *
 * The single loading spinner used across the app (inside Button's `loading`
 * state and in inline status rows like the in-chat OAuth gate). Stroke-based
 * ring + rounded-cap arc — cleaner than a filled quarter-glyph. Uses the
 * `v2-spin` keyframe (0.8s linear), which is suppressed under
 * prefers-reduced-motion.
 *
 * Props
 *   className  extra classes; REPLACES the default h-4 w-4 sizing when set
 *              (cn is concat-only, so callers restyle by replacement).
 *   label      accessible name; pass a translated string from the app.
 */
import { cn } from "./cn";

type SpinnerProps = {
  className?: string;
  label?: string;
};

export function Spinner({ className = "", label = "Loading" }: SpinnerProps) {
  return (
    <svg
      className={cn("v2-spin shrink-0", className || "h-4 w-4")}
      viewBox="0 0 24 24"
      fill="none"
      role="status"
      aria-label={label}
    >
      <circle
        cx="12"
        cy="12"
        r="9"
        stroke="currentColor"
        strokeWidth="2.5"
        className="opacity-25"
      />
      <path
        d="M21 12a9 9 0 0 0-9-9"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        className="opacity-90"
      />
    </svg>
  );
}
