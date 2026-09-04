import type { ComponentPropsWithoutRef } from "react";

import { Icon } from "./icons";
import { cn } from "../utils/cn";

type NativeSearchFieldProps = Omit<
  ComponentPropsWithoutRef<"input">,
  "aria-label" | "onChange" | "size" | "type" | "value"
> & {
  "aria-label": string;
  onChange: (value: string) => void;
  value: string;
};

const sizeClasses = {
  md: {
    clear: "right-2 h-6 w-6",
    clearIcon: "h-3.5 w-3.5",
    icon: "left-3 h-4 w-4",
    input: "h-9 rounded-[10px] pl-9 text-sm",
    inputWithClear: "pr-9",
    inputWithoutClear: "pr-3",
  },
  sm: {
    clear: "right-1.5 h-6 w-6",
    clearIcon: "h-3 w-3",
    icon: "left-2.5 h-3.5 w-3.5",
    input: "h-8 rounded-[8px] pl-8 text-xs",
    inputWithClear: "pr-8",
    inputWithoutClear: "pr-2.5",
  },
};

export type SearchFieldSize = keyof typeof sizeClasses;

type SearchFieldProps = NativeSearchFieldProps & {
  size?: SearchFieldSize;
} &
  (
    | { clearLabel: string; onClear: () => void }
    | { clearLabel?: never; onClear?: never }
  );

export function SearchField({
  "aria-label": ariaLabel,
  className = "",
  clearLabel,
  disabled = false,
  onChange,
  onClear,
  size = "md",
  value,
  ...rest
}: SearchFieldProps) {
  const showClear = Boolean(value && onClear);
  const styles = sizeClasses[size];

  return (
    <div className={cn("relative min-w-0", className)}>
      <Icon
        name="search"
        className={cn(
          "pointer-events-none absolute top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]",
          styles.icon,
        )}
      />
      <input
        {...rest}
        type="search"
        value={value}
        aria-label={ariaLabel}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value)}
        className={cn(
          "w-full border border-[var(--v2-panel-border)]",
          "bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] outline-none",
          "appearance-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]",
          "[&::-webkit-search-cancel-button]:hidden [&::-webkit-search-decoration]:hidden",
          "disabled:cursor-not-allowed disabled:opacity-50",
          styles.input,
          showClear ? styles.inputWithClear : styles.inputWithoutClear,
        )}
      />
      {showClear && (
        <button
          type="button"
          aria-label={clearLabel}
          disabled={disabled}
          onClick={onClear}
          className={cn(
            "absolute top-1/2 grid -translate-y-1/2 place-items-center rounded-md",
            "text-[var(--v2-text-faint)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
            "disabled:cursor-not-allowed disabled:opacity-50",
            styles.clear,
          )}
        >
          <Icon name="close" className={styles.clearIcon} />
        </button>
      )}
    </div>
  );
}
