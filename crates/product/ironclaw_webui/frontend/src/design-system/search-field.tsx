import type { ComponentPropsWithoutRef } from "react";

import { Icon } from "./icons";
import { cn } from "../utils/cn";

type NativeSearchFieldProps = Omit<
  ComponentPropsWithoutRef<"input">,
  "aria-label" | "onChange" | "type" | "value"
> & {
  "aria-label": string;
  onChange: (value: string) => void;
  value: string;
};

type SearchFieldProps = NativeSearchFieldProps &
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
  value,
  ...rest
}: SearchFieldProps) {
  const showClear = Boolean(value && onClear);

  return (
    <div className={cn("relative min-w-0", className)}>
      <Icon
        name="search"
        className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--v2-text-faint)]"
      />
      <input
        {...rest}
        type="search"
        value={value}
        aria-label={ariaLabel}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value)}
        className={cn(
          "h-9 w-full rounded-[10px] border border-[var(--v2-panel-border)]",
          "bg-[var(--v2-input-bg)] pl-9 text-sm text-[var(--v2-text-strong)] outline-none",
          "appearance-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]",
          "[&::-webkit-search-cancel-button]:hidden [&::-webkit-search-decoration]:hidden",
          "disabled:cursor-not-allowed disabled:opacity-50",
          showClear ? "pr-9" : "pr-3",
        )}
      />
      {showClear && (
        <button
          type="button"
          aria-label={clearLabel}
          disabled={disabled}
          onClick={onClear}
          className={cn(
            "absolute right-2 top-1/2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded-md",
            "text-[var(--v2-text-faint)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
            "disabled:cursor-not-allowed disabled:opacity-50",
          )}
        >
          <Icon name="close" className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}
