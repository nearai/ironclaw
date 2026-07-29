/**
 * InputOTP
 *
 * One-time-code entry rendered as N single-character cells. Lightweight own
 * implementation (no input-otp dep): each cell is a real <input> so focus,
 * selection, and mobile keyboards behave natively.
 *
 * Behavior
 *   - typing advances to the next cell; Backspace on an empty cell moves back
 *   - ArrowLeft/ArrowRight move between cells
 *   - pasting distributes the clipboard across cells from the focused one
 *   - inputMode="numeric" (default) strips non-digits
 *
 * Props
 *   length      number of cells (default 6)
 *   value       controlled code string
 *   onChange    (value: string) => void
 *   onComplete  fired when all cells are filled
 *   label       group aria-label (default "One-time code"); each cell
 *               announces "Digit N of M"
 */
import React from "react";
import { cn } from "../primitives/cn";

type InputOTPProps = {
  value: string;
  onChange: (value: string) => void;
  length?: number;
  onComplete?: (value: string) => void;
  disabled?: boolean;
  /** "numeric" (default) restricts entry to digits. */
  inputMode?: "numeric" | "text";
  label?: string;
  autoFocus?: boolean;
  className?: string;
};

export function InputOTP({
  value,
  onChange,
  length = 6,
  onComplete,
  disabled = false,
  inputMode = "numeric",
  label = "One-time code",
  autoFocus = false,
  className,
}: InputOTPProps) {
  const cellRefs = React.useRef<(HTMLInputElement | null)[]>([]);

  const sanitize = React.useCallback(
    (raw: string) => {
      const cleaned = inputMode === "numeric" ? raw.replace(/\D/g, "") : raw.replace(/\s/g, "");
      return cleaned.slice(0, length);
    },
    [inputMode, length]
  );

  const commit = (next: string) => {
    onChange(next);
    if (next.length === length) onComplete?.(next);
  };

  const focusCell = (index: number) => {
    const clamped = Math.min(Math.max(index, 0), length - 1);
    cellRefs.current[clamped]?.focus();
    cellRefs.current[clamped]?.select();
  };

  const handleCellChange = (index: number, raw: string) => {
    const char = sanitize(raw).slice(-1);
    if (!char) return;
    const chars = value.split("").slice(0, length);
    chars[index] = char;
    // Fill any holes before the edited cell so value stays contiguous.
    const next = chars.map((c) => c ?? "").join("").slice(0, length);
    commit(next);
    if (index < length - 1) focusCell(index + 1);
  };

  const handleKeyDown = (index: number, event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Backspace") {
      event.preventDefault();
      if (value[index]) {
        commit(value.slice(0, index) + value.slice(index + 1));
      } else if (index > 0) {
        commit(value.slice(0, index - 1) + value.slice(index));
        focusCell(index - 1);
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      focusCell(index - 1);
      return;
    }
    if (event.key === "ArrowRight") {
      event.preventDefault();
      focusCell(index + 1);
    }
  };

  const handlePaste = (index: number, event: React.ClipboardEvent<HTMLInputElement>) => {
    event.preventDefault();
    const pasted = sanitize(event.clipboardData.getData("text"));
    if (!pasted) return;
    const next = (value.slice(0, index) + pasted).slice(0, length);
    commit(next);
    focusCell(next.length >= length ? length - 1 : next.length);
  };

  return (
    <div role="group" aria-label={label} className={cn("flex items-center gap-2", className)}>
      {Array.from({ length }, (_cell, index) => (
        <input
          key={index}
          ref={(node) => {
            cellRefs.current[index] = node;
          }}
          type="text"
          inputMode={inputMode}
          autoComplete={index === 0 ? "one-time-code" : "off"}
          maxLength={1}
          value={value[index] ?? ""}
          disabled={disabled}
          autoFocus={autoFocus && index === 0}
          aria-label={`Digit ${index + 1} of ${length}`}
          onChange={(event) => handleCellChange(index, event.target.value)}
          onKeyDown={(event) => handleKeyDown(index, event)}
          onPaste={(event) => handlePaste(index, event)}
          onFocus={(event) => event.target.select()}
          className={cn(
            "h-11 w-9 rounded-[10px] border text-center text-ui-lg font-medium",
            "border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)]",
            "outline-none transition-colors",
            "focus:border-[var(--v2-accent)]",
            "focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",
            "disabled:cursor-not-allowed disabled:opacity-50"
          )}
        />
      ))}
    </div>
  );
}
