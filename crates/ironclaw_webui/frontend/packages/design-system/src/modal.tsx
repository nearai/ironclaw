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
 *
 * Motion is CSS-only (`.v2-modal-scrim` / `.v2-modal-panel` keyframes in
 * tokens.css, driven by data-state): the framer-motion runtime is ~50KB
 * gzip and Modal sits in the chat route's initial import graph, so it must
 * not pull an animation engine (see scripts/check-bundle-budgets.ts).
 * Exit animations work by holding the tree mounted for the exit duration
 * (`useExitPresence`) with data-state="closed" before unmounting.
 */
import * as Dialog from "@radix-ui/react-dialog";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useDesignSystemT } from "./i18n";
import { cn } from "./cn";
import { Icon } from "./icons";
import { MOTION_DURATION } from "./motion";

/* ─── Exit presence ───────────────────────────────────────────────── */

const EXIT_MS = MOTION_DURATION.exit * 1000;

/** Keeps the subtree mounted for `exitMs` after `open` flips false so the
 *  CSS exit animation can play. Returns whether to render at all. */
function useExitPresence(open: boolean, exitMs: number): boolean {
  const [exiting, setExiting] = useState(false);
  const wasOpen = useRef(open);

  useEffect(() => {
    if (open) {
      wasOpen.current = true;
      setExiting(false);
      return;
    }
    if (!wasOpen.current) return;
    wasOpen.current = false;
    setExiting(true);
    const id = window.setTimeout(() => setExiting(false), exitMs);
    return () => window.clearTimeout(id);
  }, [open, exitMs]);

  return open || exiting;
}

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
  const present = useExitPresence(open, EXIT_MS);
  const dataState = open ? "open" : "closed";

  return (
    <Dialog.Root
      open={present}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onClose?.();
      }}
    >
      {present ? (
        <div
          key="modal"
          className="fixed inset-0 z-50 flex items-end justify-center p-4 sm:items-center"
        >
          <Dialog.Overlay asChild>
            <div
              data-state={dataState}
              className="v2-modal-scrim absolute inset-0 bg-[var(--v2-scrim)] backdrop-blur-sm"
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
            <div
              data-state={dataState}
              className={cn(
                "v2-modal-panel relative z-10 w-full",
                "bg-[var(--v2-card-bg)] border border-[var(--v2-panel-border)]",
                "shadow-[var(--v2-shadow-modal)]",
                "rounded-[var(--v2-radius-2xl)]",
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
            </div>
          </Dialog.Content>
        </div>
      ) : null}
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
        <h2 className="text-[1.1rem] font-medium tracking-[-0.02em] text-[var(--v2-text-strong)] md:text-[1.2rem]">
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
