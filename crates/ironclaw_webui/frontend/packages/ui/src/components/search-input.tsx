/**
 * SearchInput
 *
 * Search field with a leading glyph, sr-only label and an optional clear
 * button. Promoted from the settings toolbar search — the best-designed of
 * the app's many hand-rolled `input[type=search]` rows — restyled onto the
 * shared Input tokens.
 *
 * Props
 *   label      accessible name (sr-only; also the default placeholder)
 *   onClear    optional — shows the trailing clear button while value is set
 *   clearLabel aria-label for the clear button (pass a translated string)
 *   size       Input size ("sm" fits toolbars, default)
 *   className  wrapper additions (flex sizing, …)
 *   ...rest    forwarded to the <input> (value, onChange, placeholder, …)
 */
import type { ComponentPropsWithoutRef, ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../icons/icon";
import { Input, type InputSize } from "./input";

type SearchInputProps = {
  label: ReactNode;
  onClear?: () => void;
  clearLabel?: string;
  size?: InputSize;
  className?: string;
} & Omit<ComponentPropsWithoutRef<"input">, "size" | "className">;

export function SearchInput({
  label,
  onClear,
  clearLabel,
  size = "sm",
  className = "",
  ...rest
}: SearchInputProps) {
  const showClear = Boolean(onClear && rest.value);
  return (
    <label className={cn("relative block min-w-0", className)}>
      <span className="sr-only">{label}</span>
      <Icon
        name="search"
        className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--v2-text-faint)]"
      />
      <Input type="search" size={size} className="pl-9 pr-9" {...rest} />
      {showClear &&
        (<button
          type="button"
          onClick={onClear}
          aria-label={clearLabel}
          className="absolute right-2 top-1/2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded-md
            text-[var(--v2-text-faint)] transition-colors
            hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]
            focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
        >
          <Icon name="close" className="h-3.5 w-3.5" />
        </button>)}
    </label>
  );
}
