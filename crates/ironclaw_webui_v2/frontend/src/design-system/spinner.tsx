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
 *   className  extra classes (e.g. sizing / color); defaults to h-4 w-4.
 */
import { cn } from "../utils/cn";

export function Spinner({ className = "" } = {}) {
  return (
    <svg
      className={cn("v2-spin shrink-0", className || "h-4 w-4")}
      viewBox="0 0 24 24"
      fill="none"
      role="status"
      aria-label="Loading"
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
