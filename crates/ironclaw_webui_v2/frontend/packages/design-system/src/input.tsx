/**
 * Form inputs
 *
 * All styling via Tailwind + CSS variables — no app.css classes.
 * Sizes and focus ring match the reference AppInput exactly:
 *   mobile  h-[44px] rounded-[14px] px-3.5 text-[13px]
 *   desktop h-[50px] rounded-[16px] px-4   text-sm
 *
 * Exports
 *   Input       — <input> wrapper
 *   Textarea    — <textarea> wrapper (auto-grows via rows prop)
 *   Select      — <select> wrapper with custom arrow
 *   Label       — <label> with consistent typography
 *   FormField   — Label + Input/children + optional error/hint
 */
import { cn } from "./cn";

/* ─── Shared base ─────────────────────────────────────────────────── */

const INPUT_BASE =
  "w-full border bg-[var(--v2-input-bg)] text-[var(--v2-text-strong)] " +
  "placeholder:text-[var(--v2-text-faint)] " +
  "border-[var(--v2-panel-border)] " +
  "outline-none " +
  // Hover feedback: a fast border tint (no movement — inputs never
  // scale). The focus ring itself is NOT transitioned: focus must land
  // instantly, especially under keyboard traversal.
  "transition-[border-color,background-color] " +
  "duration-[var(--v2-duration-fast)] ease-[var(--v2-ease-standard)] " +
  "hover:not-focus:border-[color-mix(in_srgb,var(--v2-accent)_25%,var(--v2-panel-border))] " +
  "focus:border-[var(--v2-accent)] " +
  "focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)] " +
  "disabled:cursor-not-allowed disabled:opacity-50";

/* Compact control-density scale — same --v2-control-* tokens as
   Button so mixed control rows align (see DESIGN_SYSTEM.md §4). */
const INPUT_SIZES = {
  sm: "h-[var(--v2-control-h-sm)] rounded-[var(--v2-radius-sm)] px-[var(--v2-control-px-sm)] text-[length:var(--v2-font-size-caption)]",
  md: "h-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)] px-[var(--v2-control-px-md)] text-[length:var(--v2-font-size-body-sm)]",
  lg: "h-[var(--v2-control-h-lg)] rounded-[var(--v2-radius-md)] px-[var(--v2-control-px-lg)] text-[length:var(--v2-font-size-body)]",
};

/* ─── Input ───────────────────────────────────────────────────────── */

export function Input({
  className = "",
  size = "md",
  error = false,
  ...rest
}) {
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

export function Textarea({
  className = "",
  error = false,
  rows = 4,
  ...rest
}) {
  return (
    <textarea
      rows={rows}
      className={cn(
        INPUT_BASE,
        "rounded-[var(--v2-radius-md)] px-[var(--v2-control-px-md)] py-2 text-[length:var(--v2-font-size-body-sm)]",
        "resize-y min-h-[72px]",
        error && "border-[var(--v2-danger-text)] focus:ring-[color-mix(in_srgb,var(--v2-danger-text)_28%,transparent)]",
        className
      )}
      {...rest}
    />
  );
}

/* ─── Select ──────────────────────────────────────────────────────── */

export function Select({
  children,
  className = "",
  size = "md",
  error = false,
  ...rest
}) {
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

export function Label({ children, className = "", required = false, ...rest }) {
  return (
    <label
      className={cn(
        "block text-[13px] font-medium text-[var(--v2-text-strong)] md:text-sm",
        className
      )}
      {...rest}
    >
      {children}
      {required && (<span className="ml-0.5 text-[var(--v2-danger-text)]" aria-hidden="true"> *</span>)}
    </label>
  );
}

/* ─── FormField ───────────────────────────────────────────────────── */
/**
 * Composes Label + input element + optional hint/error message.
 *
 * Usage:
 *   <${FormField} label="First name" error=${errors.firstName?.message}>
 *     <${Input} ...${register("firstName")} />
 *   <//>
 */
export function FormField({
  label,
  children,
  error = "",
  hint = "",
  required = false,
  className = "",
  htmlFor = "",
}) {
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      {label &&
        (<Label htmlFor={htmlFor} required={required}>{label}</Label>) }
      {children}
      {error &&
        (<p className="text-xs text-[var(--v2-danger-text)]" role="alert">{error}</p>)}
      {!error && hint &&
        (<p className="text-xs text-[var(--v2-text-faint)]">{hint}</p>)}
    </div>
  );
}
