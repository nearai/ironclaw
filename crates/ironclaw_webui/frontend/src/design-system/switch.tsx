import type { ComponentPropsWithoutRef } from "react";

import { cn } from "../utils/cn";

const SIZES = {
  sm: {
    track: "h-6 w-11",
    thumb: "h-5 w-5",
    checkedThumb: "translate-x-5",
    uncheckedThumb: "translate-x-0",
  },
  md: {
    track: "h-7 w-12",
    thumb: "h-5 w-5",
    checkedThumb: "translate-x-5",
    uncheckedThumb: "translate-x-1",
  },
};

type AccessibleName =
  | {
      "aria-label": string;
      "aria-labelledby"?: string;
    }
  | {
      "aria-label"?: string;
      "aria-labelledby": string;
    };

type NativeButtonProps = Omit<
  ComponentPropsWithoutRef<"button">,
  | "aria-checked"
  | "aria-label"
  | "aria-labelledby"
  | "children"
  | "onChange"
  | "onClick"
  | "role"
  | "type"
>;

type SwitchProps = NativeButtonProps &
  AccessibleName & {
    checked: boolean;
    onChange: (checked: boolean) => void;
    size?: keyof typeof SIZES;
  };

export function Switch({
  checked,
  className = "",
  disabled = false,
  onChange,
  size = "md",
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
  ...rest
}: SwitchProps) {
  const sizeClasses = SIZES[size] ?? SIZES.md;

  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      aria-labelledby={ariaLabelledBy}
      disabled={disabled}
      onClick={() => {
        if (!disabled) onChange(!checked);
      }}
      className={cn(
        "relative inline-flex shrink-0 items-center rounded-full border transition",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1",
        "focus-visible:ring-offset-[var(--v2-canvas)]",
        disabled ? "cursor-not-allowed opacity-60" : "cursor-pointer",
        checked
          ? "border-[color-mix(in_srgb,var(--v2-accent)_45%,transparent)] bg-[color-mix(in_srgb,var(--v2-accent)_22%,transparent)]"
          : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
        sizeClasses.track,
        className
      )}
      {...rest}
    >
      <span
        aria-hidden="true"
        className={cn(
          "pointer-events-none inline-block rounded-full transition",
          sizeClasses.thumb,
          checked
            ? `${sizeClasses.checkedThumb} bg-[var(--v2-accent-text)]`
            : `${sizeClasses.uncheckedThumb} bg-[var(--v2-text-muted)]`
        )}
      />
    </button>
  );
}
