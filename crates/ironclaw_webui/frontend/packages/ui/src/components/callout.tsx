/**
 * Callout
 *
 * Inline feedback banner (page-level errors, success/info notices).
 * Consolidates the hand-rolled banner markup that pages built from
 * `border-red-400/30 bg-red-500/10 …` etc. Tone colors reproduce the
 * theme-resolved values those legacy utility classes rendered as.
 *
 * Props
 *   tone         "info" (default) | "success" | "danger"
 *   onDismiss    optional — renders a text dismiss button
 *   dismissLabel label for the dismiss button (pass a translated string)
 *   className    layout additions (margins, …)
 *   ...rest      forwarded to the root div (role="alert", data-testid, …)
 */
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";

const TONES = {
  info:
    "border-[color-mix(in_srgb,var(--v2-accent)_44%,var(--v2-panel-border))] " +
    "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",
  success:
    "border-[color-mix(in_oklab,var(--v2-accent-text)_30%,transparent)] " +
    "bg-[color-mix(in_oklab,var(--v2-accent-text)_10%,transparent)] text-[var(--v2-accent-text)]",
  danger:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] " +
    "bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",
};

type CalloutProps = {
  tone?: keyof typeof TONES;
  children?: ReactNode;
  onDismiss?: () => void;
  dismissLabel?: ReactNode;
  className?: string;
  [key: string]: unknown;
};

export function Callout({
  tone = "info",
  children,
  onDismiss = undefined,
  dismissLabel = undefined,
  className = "",
  ...rest
}: CalloutProps) {
  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",
        TONES[tone] ?? TONES.info,
        className
      )}
      {...rest}
    >
      <span className="min-w-0 flex-1">{children}</span>
      {onDismiss &&
        (<button
          type="button"
          onClick={onDismiss}
          className="shrink-0 opacity-70 hover:opacity-100"
        >
          {dismissLabel}
        </button>)}
    </div>
  );
}
