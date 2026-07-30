/**
 * FormField
 *
 * Composes Label + a form control + optional hint/error message. Lives in
 * composites because it assembles components (Label + Input/Select/…) rather
 * than wrapping a single element.
 *
 * Props
 *   label     Label content
 *   htmlFor   id of the control the label points at
 *   required  renders the Label asterisk
 *   error     message shown below the control (role="alert"); wins over hint
 *   hint      muted helper text shown when there is no error
 *   className layout additions
 *   children  the control(s)
 */
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Label } from "../components/input";

type FormFieldProps = {
  label?: ReactNode;
  children?: ReactNode;
  error?: ReactNode;
  hint?: ReactNode;
  required?: boolean;
  className?: string;
  htmlFor?: string;
};

export function FormField({
  label,
  children,
  error = "",
  hint = "",
  required = false,
  className = "",
  htmlFor = "",
}: FormFieldProps) {
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
