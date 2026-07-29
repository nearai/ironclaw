/**
 * Modal
 *
 * Accessible dialog with backdrop.  Pure Tailwind — no app.css classes.
 * Renders into a portal-like fixed overlay; body scroll is locked while open.
 *
 * Props
 *   open      boolean
 *   onClose   () => void  — called on backdrop click or Escape key
 *   title     string
 *   size      "sm" | "md" (default) | "lg" | "xl" | "full"
 *   className string — applied to the dialog panel
 *   children
 *
 * Sub-components (all optional)
 *   <ModalHeader>  — renders title + close button row
 *   <ModalBody>    — scrollable content area
 *   <ModalFooter>  — action button row with top divider
 */
import React, { type ReactNode } from "react";
import { useUiText } from "../theme/ui-text";
import { cn } from "../primitives/cn";
import { Icon } from "../primitives/icon";

/* ─── Size ────────────────────────────────────────────────────────── */

const SIZES = {
  sm:   "max-w-sm",
  md:   "max-w-lg",
  lg:   "max-w-2xl",
  xl:   "max-w-4xl",
  full: "max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]",
};

/* ─── Modal ───────────────────────────────────────────────────────── */

type ModalProps = {
  open: boolean;
  onClose?: () => void;
  title?: ReactNode;
  size?: keyof typeof SIZES;
  className?: string;
  /** Close-button aria-label; defaults to the UiTextProvider string. */
  closeLabel?: string;
  children?: ReactNode;
};

export function Modal({
  open,
  onClose,
  title,
  size = "md",
  className = "",
  closeLabel,
  children,
}: ModalProps) {
  /* Lock body scroll when open */
  React.useEffect(() => {
    if (!open) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, [open]);

  /* Close on Escape */
  React.useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose?.();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center p-4 sm:items-center"
      aria-modal="true"
      aria-label={typeof title === "string" ? title : undefined}
      role="dialog"
    >
      {/* Dim layer */}
      <div
        className="absolute inset-0 bg-black/55 backdrop-blur-sm"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Panel */}
      <div
        className={cn(
          "relative z-10 w-full",
          "bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]",
          "shadow-[0_24px_60px_rgba(0,0,0,0.35)]",
          "rounded-[1.5rem]",
          "flex flex-col max-h-[90dvh] overflow-hidden",
          SIZES[size] ?? SIZES.md,
          className
        )}
      >
        {title
          ? (<ModalHeader onClose={onClose} closeLabel={closeLabel}>{title}</ModalHeader>) : null}
        {children}
      </div>
    </div>
  );
}

/* ─── ModalHeader ─────────────────────────────────────────────────── */

type ModalHeaderProps = {
  children?: ReactNode;
  onClose?: () => void;
  className?: string;
  closeLabel?: string;
};

export function ModalHeader({ children, onClose, className = "", closeLabel }: ModalHeaderProps) {
  const uiText = useUiText();
  const effectiveCloseLabel = closeLabel || uiText.close;
  return (
    <div
      className={cn(
        "flex shrink-0 items-center justify-between gap-4",
        "px-5 py-4 md:px-7 md:py-5",
        "border-b border-[var(--v2-panel-border)]",
        className
      )}
    >
      <h2
        className="text-[1.1rem] font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)] md:text-[1.2rem]"
      >
        {children}
      </h2>
      {onClose &&
        (
          <button
            type="button"
            onClick={onClose}
            aria-label={effectiveCloseLabel}
            className="grid h-8 w-8 shrink-0 place-items-center rounded-[10px]
              border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]
              text-[var(--v2-text-muted)] transition-colors
              hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]
              active:bg-[color-mix(in_srgb,var(--v2-text-strong)_10%,var(--v2-surface-muted))]
              focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
          >
            <Icon name="close" className="h-4 w-4" />
          </button>
        )}
    </div>
  );
}

/* ─── ModalBody ───────────────────────────────────────────────────── */

type ModalSectionProps = {
  children?: ReactNode;
  className?: string;
};

export function ModalBody({ children, className = "" }: ModalSectionProps) {
  return (
    <div className={cn("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5", className)}>
      {children}
    </div>
  );
}

/* ─── ModalFooter ─────────────────────────────────────────────────── */

export function ModalFooter({ children, className = "" }: ModalSectionProps) {
  return (
    <div
      className={cn(
        "shrink-0 flex items-center justify-end gap-3 flex-wrap",
        "px-5 py-4 md:px-7 md:py-5",
        "border-t border-[var(--v2-panel-border)]",
        className
      )}
    >
      {children}
    </div>
  );
}
