/**
 * Callout
 *
 * Inline feedback banner (page-level errors, warnings, success/info notices).
 * Consolidates the hand-rolled banner markup that pages built from
 * `border-red-400/30 bg-red-500/10 …` etc. Tone recipes share the Badge
 * token vocabulary so chips and banners agree in both themes.
 *
 * Props
 *   tone         "info" (default) | "success" | "warning" | "danger"
 *   title        optional bold first line; children render below it
 *   actions      optional trailing controls (e.g. a retry Button)
 *   onDismiss    optional — renders a text dismiss button
 *   dismissLabel label for the dismiss button (pass a translated string)
 *   className    layout additions (margins, …)
 *   ...rest      forwarded to the root div (data-testid, …)
 *
 * The root div defaults to a live-region role — "alert" for danger,
 * "status" otherwise — so dynamically mounted callouts are announced by
 * assistive tech. Pass an explicit `role` to override.
 */
import type { ComponentPropsWithoutRef, ReactNode } from "react";
import { cn } from "../primitives/cn";

const TONES = {
  info:
    "border-[color-mix(in_srgb,var(--v2-accent)_44%,var(--v2-panel-border))] " +
    "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",
  success:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] " +
    "bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
  warning:
    "border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] " +
    "bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",
  danger:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] " +
    "bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",
};

export type CalloutTone = keyof typeof TONES;

type CalloutProps = {
  tone?: CalloutTone;
  title?: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  onDismiss?: () => void;
  dismissLabel?: ReactNode;
  className?: string;
} & Omit<ComponentPropsWithoutRef<"div">, "className" | "children" | "title">;

export function Callout({
  tone = "info",
  title,
  children,
  actions,
  onDismiss,
  dismissLabel,
  className = "",
  ...rest
}: CalloutProps) {
  return (
    <div
      role={tone === "danger" ? "alert" : "status"}
      className={cn(
        "flex flex-wrap items-center gap-3 rounded-xl border px-4 py-3 text-sm",
        TONES[tone] ?? TONES.info,
        className
      )}
      {...rest}
    >
      <div className="min-w-0 flex-1">
        {title && (<div className="font-semibold">{title}</div>)}
        {children != null &&
          (<div className={title ? "mt-0.5 text-xs leading-5 opacity-80" : undefined}>{children}</div>)}
      </div>
      {actions && (<div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>)}
      {onDismiss &&
        (<button
          type="button"
          onClick={onDismiss}
          className="shrink-0 rounded-[6px] opacity-70 transition-opacity hover:opacity-100 active:opacity-100
            focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
        >
          {dismissLabel}
        </button>)}
    </div>
  );
}
