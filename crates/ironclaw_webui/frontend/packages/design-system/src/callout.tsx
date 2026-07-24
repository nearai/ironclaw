/**
 * Callout — inline notice panel for guidance, caveats, and status context.
 *
 * Tones follow the STATUS_CANON semantics (tokens.ts) plus `accent` for
 * product highlights. The icon is chosen by tone and can be overridden or
 * removed. Content is a slot: pass any children, keep copy per the Voice &
 * copy rules (calm, plain, no alarm-speak).
 *
 * Props
 *   tone      "info" (default) | "accent" | "success" | "warning" | "danger" | "muted"
 *   title     optional bold lead line
 *   icon      icon name override; pass null to render no icon
 *   className layout/spacing additions
 *   children  body content
 */
import type { ReactNode } from "react";
import { cn } from "./cn";
import { Icon } from "./icons";

export type CalloutTone = "info" | "accent" | "success" | "warning" | "danger" | "muted";

const TONE_CLASSES: Record<CalloutTone, string> = {
  info: "border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] bg-[var(--v2-info-soft)]",
  accent:
    "border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]",
  success:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)]",
  warning:
    "border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)]",
  danger:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)]",
  muted: "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
};

const TONE_TEXT: Record<CalloutTone, string> = {
  info: "text-[var(--v2-info-text)]",
  accent: "text-[var(--v2-accent-text)]",
  success: "text-[var(--v2-positive-text)]",
  warning: "text-[var(--v2-warning-text)]",
  danger: "text-[var(--v2-danger-text)]",
  muted: "text-[var(--v2-text-muted)]",
};

const TONE_ICON: Record<CalloutTone, string> = {
  info: "flag",
  accent: "spark",
  success: "check",
  warning: "bell",
  danger: "shield",
  muted: "pin",
};

export interface CalloutProps {
  tone?: CalloutTone;
  title?: ReactNode;
  /** Icon name override; pass `null` to render without an icon. */
  icon?: string | null;
  className?: string;
  children?: ReactNode;
}

export function Callout({ tone = "info", title, icon, className = "", children }: CalloutProps) {
  const resolvedTone = TONE_CLASSES[tone] ? tone : "info";
  const iconName = icon === undefined ? TONE_ICON[resolvedTone] : icon;
  return (
    <aside
      role="note"
      className={cn(
        "grid gap-3 rounded-[var(--v2-radius-lg)] border px-5 py-4",
        iconName ? "grid-cols-[1.25rem_minmax(0,1fr)]" : "grid-cols-1",
        TONE_CLASSES[resolvedTone],
        className
      )}
    >
      {iconName && (
        <span className={cn("mt-0.5", TONE_TEXT[resolvedTone])}>
          <Icon name={iconName} className="h-[1.05rem] w-[1.05rem]" />
        </span>
      )}
      <div className="min-w-0 text-sm leading-6 text-[var(--v2-text-strong)]">
        {title && (
          <strong className={cn("mb-0.5 block font-medium", TONE_TEXT[resolvedTone])}>
            {title}
          </strong>
        )}
        {children}
      </div>
    </aside>
  );
}
