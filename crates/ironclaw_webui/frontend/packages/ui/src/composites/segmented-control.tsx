/**
 * SegmentedControl
 *
 * Bordered group of mutually exclusive filter buttons. Promoted from the
 * automations list filter — the app's best hand-rolled segmented group
 * (role="group", aria-pressed, token colors) — so jobs/routines/missions
 * stop expressing the same idea as bare <select>s or ad-hoc pill rows.
 *
 * Props
 *   options      [{ value, label, disabled? }]
 *   value        selected option value
 *   onChange     (value) => void
 *   label        group aria-label (pass a translated string)
 *   optionTestId optional data-testid stamped on every option button;
 *                each button also carries data-value for e2e filtering
 *   className    layout additions
 */
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";

export type SegmentedControlOption = {
  value: string;
  label: ReactNode;
  disabled?: boolean;
};

type SegmentedControlProps = {
  options: SegmentedControlOption[];
  value: string;
  onChange: (value: string) => void;
  label: string;
  optionTestId?: string;
  className?: string;
};

export function SegmentedControl({
  options,
  value,
  onChange,
  label,
  optionTestId,
  className = "",
}: SegmentedControlProps) {
  return (
    <div
      role="group"
      aria-label={label}
      className={cn(
        "inline-flex max-w-full overflow-x-auto rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
        className
      )}
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          data-testid={optionTestId}
          data-value={option.value}
          aria-pressed={value === option.value}
          disabled={option.disabled}
          onClick={() => onChange(option.value)}
          className={cn(
            "min-h-9 shrink-0 whitespace-nowrap px-3 py-2 text-xs font-semibold leading-tight transition-colors",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--v2-focus-ring)]",
            "disabled:cursor-not-allowed disabled:opacity-50",
            value === option.value
              ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
              : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
