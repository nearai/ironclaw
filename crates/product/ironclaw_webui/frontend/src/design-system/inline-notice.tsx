import type { ComponentPropsWithoutRef, ReactNode } from "react";

import { cn } from "../utils/cn";
import { Icon } from "./icons";

export type InlineNoticeTone = "info" | "success" | "warning" | "danger";

const TONE_STYLES: Record<InlineNoticeTone, string> = {
  info:
    "border-[color-mix(in_srgb,var(--v2-info-text)_30%,var(--v2-panel-border))] " +
    "bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",
  success:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] " +
    "bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
  warning:
    "border-[color-mix(in_srgb,var(--v2-warning-text)_30%,var(--v2-panel-border))] " +
    "bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",
  danger:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_30%,var(--v2-panel-border))] " +
    "bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",
};

const TONE_ICONS: Record<InlineNoticeTone, ComponentPropsWithoutRef<typeof Icon>["name"]> = {
  info: "bolt",
  success: "check",
  warning: "alert",
  danger: "alert",
};

type InlineNoticeBaseProps = Omit<
  ComponentPropsWithoutRef<"div">,
  "children" | "role"
> & {
  action?: ReactNode;
  children: ReactNode;
  role: "alert" | "status";
  tone: InlineNoticeTone;
};

type InlineNoticeDismissProps =
  | {
      dismissLabel: string;
      onDismiss: () => void;
    }
  | {
      dismissLabel?: never;
      onDismiss?: never;
    };

export type InlineNoticeProps = InlineNoticeBaseProps & InlineNoticeDismissProps;

export function InlineNotice({
  action,
  children,
  className,
  dismissLabel,
  onDismiss,
  role,
  tone,
  ...rest
}: InlineNoticeProps) {
  return (
    <div
      {...rest}
      role={role}
      data-tone={tone}
      className={cn(
        "flex items-start gap-3 rounded-xl border px-4 py-3 text-sm",
        TONE_STYLES[tone],
        className,
      )}
    >
      <Icon
        name={TONE_ICONS[tone]}
        className="mt-0.5 h-4 w-4 shrink-0"
      />
      <div className="min-w-0 flex-1">{children}</div>
      {action && <div className="shrink-0 self-center">{action}</div>}
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label={dismissLabel}
          className="shrink-0 self-center rounded-md p-1 opacity-70 transition-opacity hover:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-current"
        >
          <Icon name="close" className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}
