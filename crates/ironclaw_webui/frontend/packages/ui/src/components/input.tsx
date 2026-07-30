/**
 * Form inputs
 *
 * All styling via Tailwind + CSS variables — no app.css classes.
 * Sizes and focus ring match the reference AppInput exactly:
 *   mobile  h-[44px] rounded-[14px] px-3.5 text-ui
 *   desktop h-[50px] rounded-[16px] px-4   text-ui
 *
 * Exports
 *   Input       — <input> wrapper
 *   Textarea    — <textarea> wrapper (auto-grows via rows prop)
 *   Select      — <select> wrapper with custom arrow
 *   Label       — <label> with consistent typography
 */
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "../primitives/cn";

/* ─── Shared base ─────────────────────────────────────────────────── */

const INPUT_BASE =
  "w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] " +
  "placeholder:text-[var(--v2-text-faint)] " +
  "border-[var(--v2-panel-border)] " +
  "outline-none transition-colors " +
  "focus:border-[var(--v2-accent)] " +
  "focus:ring-2 focus:ring-[var(--v2-focus-ring)] " +
  "disabled:cursor-not-allowed disabled:opacity-50";

/* Sizes mirroring reference AppInput */
const INPUT_SIZES = {
  sm: "h-9 rounded-[10px] px-3 text-ui-sm",
  md: "h-[44px] rounded-[14px] px-3.5 text-ui md:h-[50px] md:rounded-[16px] md:px-4",
  lg: "h-[54px] rounded-[18px] px-4 text-ui-lg",
};

export type InputSize = keyof typeof INPUT_SIZES;

/* ─── Input ───────────────────────────────────────────────────────── */

type InputProps = {
  size?: InputSize;
  error?: boolean;
} & Omit<ComponentPropsWithoutRef<"input">, "size">;

export function Input({
  className = "",
  size = "md",
  error = false,
  ...rest
}: InputProps) {
  return (
    <input
      className={cn(
        INPUT_BASE,
        INPUT_SIZES[size] ?? INPUT_SIZES.md,
        error && "border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",
        className
      )}
      {...rest}
    />
  );
}

/* ─── Textarea ────────────────────────────────────────────────────── */

type TextareaProps = {
  error?: boolean;
} & ComponentPropsWithoutRef<"textarea">;

export function Textarea({
  className = "",
  error = false,
  rows = 4,
  ...rest
}: TextareaProps) {
  return (
    <textarea
      rows={rows}
      className={cn(
        INPUT_BASE,
        "rounded-[14px] px-3.5 py-3 text-ui md:rounded-[16px] md:px-4",
        "resize-y min-h-[80px]",
        error && "border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",
        className
      )}
      {...rest}
    />
  );
}

/* ─── Select ──────────────────────────────────────────────────────── */

type SelectProps = {
  size?: InputSize;
  error?: boolean;
} & Omit<ComponentPropsWithoutRef<"select">, "size">;

export function Select({
  children,
  className = "",
  size = "md",
  error = false,
  ...rest
}: SelectProps) {
  return (
    <div className="relative w-full">
      <select
        className={cn(
          INPUT_BASE,
          INPUT_SIZES[size] ?? INPUT_SIZES.md,
          "appearance-none pr-9 cursor-pointer",
          error && "border-[var(--v2-danger-text)]",
          className
        )}
        {...rest}
      >
        {children}
      </select>
      {/* Caret arrow */}
      <span
        aria-hidden="true"
        className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[var(--v2-text-faint)]"
      >
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none"
          stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="M2.5 4.5 6 8l3.5-3.5" />
        </svg>
      </span>
    </div>
  );
}

/* ─── Label ───────────────────────────────────────────────────────── */

type LabelProps = {
  required?: boolean;
} & ComponentPropsWithoutRef<"label">;

export function Label({ children, className = "", required = false, ...rest }: LabelProps) {
  return (
    <label
      className={cn(
        "block text-ui font-medium text-[var(--v2-text-strong)]",
        className
      )}
      {...rest}
    >
      {children}
      {required && (<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>)}
    </label>
  );
}

/* FormField (Label + control + hint/error) lives in composites/form-field —
   it assembles components, which places it a layer above these controls. */
