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
import React from "react";
import { AnimatePresence, motion } from "motion/react";
import { useT } from "../lib/i18n";
import { cn } from "../utils/cn";
import { Icon } from "./icons";
import { MOTION_DURATION, MOTION_EASE_OUT, useReducedMotion } from "./motion";

/* ─── Size ────────────────────────────────────────────────────────── */

const SIZES = {
  sm:   "max-w-sm",
  md:   "max-w-lg",
  lg:   "max-w-2xl",
  xl:   "max-w-4xl",
  full: "max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]",
};

/* ─── Modal ───────────────────────────────────────────────────────── */

export function Modal({
  open,
  onClose,
  title,
  size = "md",
  className = "",
  closeLabel,
  children,
}) {
  const reducedMotion = useReducedMotion();

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
    const handler = (e) => {
      if (e.key === "Escape") onClose?.();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  /* Modal vocabulary: the scrim fades while the panel scales from 0.96
     and settles up from an 8px offset — center-origin on purpose (modals
     are not anchored to a trigger, unlike menus/popovers). Exit is
     quicker than entry so dismissal feels immediate. Reduced motion
     keeps the fades and drops the movement. AnimatePresence keeps the
     tree mounted through the exit. */
  const exitTransition = {
    duration: MOTION_DURATION.exit,
    ease: MOTION_EASE_OUT,
  };

  return (
    <AnimatePresence>
      {open && (
        <div
          className="fixed inset-0 z-50 flex items-end justify-center p-4 sm:items-center"
          aria-modal="true"
          aria-label={typeof title === "string" ? title : undefined}
          role="dialog"
        >
          {/* Dim layer */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0, transition: exitTransition }}
            transition={{ duration: MOTION_DURATION.base, ease: "easeOut" }}
            className="absolute inset-0 bg-[var(--v2-scrim)] backdrop-blur-sm"
            onClick={onClose}
            aria-hidden="true"
          />

          {/* Panel */}
          <motion.div
            initial={
              reducedMotion ? { opacity: 0 } : { opacity: 0, scale: 0.96, y: 8 }
            }
            animate={reducedMotion ? { opacity: 1 } : { opacity: 1, scale: 1, y: 0 }}
            exit={{
              opacity: 0,
              ...(reducedMotion ? {} : { scale: 0.98 }),
              transition: exitTransition,
            }}
            transition={{ duration: MOTION_DURATION.base, ease: MOTION_EASE_OUT }}
            className={cn(
              "relative z-10 w-full",
              "bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]",
              "shadow-[var(--v2-shadow-modal)]",
              "rounded-[1.5rem]",
              "flex flex-col max-h-[90dvh] overflow-hidden",
              SIZES[size] ?? SIZES.md,
              className
            )}
          >
            {title
              ? (<ModalHeader onClose={onClose} closeLabel={closeLabel}>{title}</ModalHeader>) : null}
            {children}
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}

/* ─── ModalHeader ─────────────────────────────────────────────────── */

export function ModalHeader({ children, onClose, className = "", closeLabel }) {
  const t = useT();
  const effectiveCloseLabel = closeLabel || t("common.close");
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
              text-[var(--v2-text-muted)]
              hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          >
            <Icon name="close" className="h-4 w-4" />
          </button>
        )}
    </div>
  );
}

/* ─── ModalBody ───────────────────────────────────────────────────── */

export function ModalBody({ children, className = "" }) {
  return (
    <div className={cn("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5", className)}>
      {children}
    </div>
  );
}

/* ─── ModalFooter ─────────────────────────────────────────────────── */

export function ModalFooter({ children, className = "" }) {
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
