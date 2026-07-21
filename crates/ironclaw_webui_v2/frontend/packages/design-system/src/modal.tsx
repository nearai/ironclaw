/**
 * Modal
 *
 * Accessible dialog built on `@radix-ui/react-dialog` (shadcn pattern) with
 * IronClaw surfaces + restrained motion. Public API is unchanged:
 *   open / onClose / title / size / className / closeLabel / children
 * plus ModalHeader / ModalBody / ModalFooter.
 *
 * Portal is intentionally omitted so SSR / renderToStaticMarkup consumers
 * (ConfirmDialog tests) still see the dialog markup when `open`.
 */
import * as Dialog from "@radix-ui/react-dialog";
import { AnimatePresence, motion } from "motion/react";
import type { ReactNode } from "react";
import { useDesignSystemT } from "./i18n";
import { cn } from "./cn";
import { Icon } from "./icons";
import { MOTION_DURATION, MOTION_EASE_OUT, useReducedMotion } from "./motion";

/* ─── Size ────────────────────────────────────────────────────────── */

const SIZES = {
  sm: "max-w-sm",
  md: "max-w-lg",
  lg: "max-w-2xl",
  xl: "max-w-4xl",
  full: "max-w-[calc(100vw-2rem)] max-h-[calc(100dvh-2rem)]",
};

type ModalSize = keyof typeof SIZES;

type ModalProps = {
  open: boolean;
  onClose?: () => void;
  title?: ReactNode;
  size?: ModalSize;
  className?: string;
  closeLabel?: string;
  children?: ReactNode;
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
}: ModalProps) {
  const reducedMotion = useReducedMotion();

  const exitTransition = {
    duration: MOTION_DURATION.exit,
    ease: MOTION_EASE_OUT,
  };

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onClose?.();
      }}
    >
      <AnimatePresence>
        {open ? (
          <div
            key="modal"
            className="fixed inset-0 z-50 flex items-end justify-center p-4 sm:items-center"
          >
            <Dialog.Overlay asChild>
              <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0, transition: exitTransition }}
                transition={{ duration: MOTION_DURATION.base, ease: "easeOut" }}
                className="absolute inset-0 bg-[var(--v2-scrim)] backdrop-blur-sm"
                aria-hidden="true"
              />
            </Dialog.Overlay>

            <Dialog.Content
              asChild
              aria-modal="true"
              aria-label={typeof title === "string" ? title : undefined}
              onEscapeKeyDown={(event) => {
                if (!onClose) event.preventDefault();
              }}
              onPointerDownOutside={(event) => {
                if (!onClose) {
                  event.preventDefault();
                  return;
                }
                onClose();
              }}
              onInteractOutside={(event) => {
                if (!onClose) event.preventDefault();
              }}
            >
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
                  "focus:outline-none",
                  SIZES[size] ?? SIZES.md,
                  className
                )}
              >
                {title ? (
                  <ModalHeader onClose={onClose} closeLabel={closeLabel}>
                    {title}
                  </ModalHeader>
                ) : null}
                {children}
              </motion.div>
            </Dialog.Content>
          </div>
        ) : null}
      </AnimatePresence>
    </Dialog.Root>
  );
}

/* ─── ModalHeader ─────────────────────────────────────────────────── */

type ModalHeaderProps = {
  children?: ReactNode;
  onClose?: () => void;
  className?: string;
  closeLabel?: string;
};

export function ModalHeader({
  children,
  onClose,
  className = "",
  closeLabel,
}: ModalHeaderProps) {
  const t = useDesignSystemT();
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
      <Dialog.Title asChild>
        <h2 className="text-[1.1rem] font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)] md:text-[1.2rem]">
          {children}
        </h2>
      </Dialog.Title>
      {onClose ? (
        <Dialog.Close asChild>
          <button
            type="button"
            onClick={onClose}
            aria-label={effectiveCloseLabel}
            className="grid h-8 w-8 shrink-0 place-items-center rounded-[10px]
              border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]
              text-[var(--v2-text-muted)]
              transition-[background,color,scale] duration-[var(--v2-duration-fast)]
              ease-[var(--v2-ease-standard)] active:scale-[0.97]
              hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]
              focus-visible:outline-none focus-visible:ring-2
              focus-visible:ring-[var(--v2-accent)]/50"
          >
            <Icon name="close" className="h-4 w-4" />
          </button>
        </Dialog.Close>
      ) : null}
    </div>
  );
}

/* ─── ModalBody ───────────────────────────────────────────────────── */

export function ModalBody({
  children,
  className = "",
}: {
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex-1 overflow-y-auto px-5 py-4 md:px-7 md:py-5", className)}>
      {children}
    </div>
  );
}

/* ─── ModalFooter ─────────────────────────────────────────────────── */

export function ModalFooter({
  children,
  className = "",
}: {
  children?: ReactNode;
  className?: string;
}) {
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
