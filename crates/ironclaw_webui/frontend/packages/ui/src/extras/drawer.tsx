/**
 * Drawer (a.k.a. Sheet)
 *
 * Edge-anchored panel with a dimmed backdrop. Hand-built on the same
 * conventions as the core Modal (fixed overlay, Escape to close, body scroll
 * lock, role="dialog") rather than adding a dialog dependency — the package's
 * static-motion policy means we don't need enter/exit choreography.
 *
 * Props
 *   open      boolean
 *   onClose   () => void — backdrop click, close button, or Escape
 *   side      "right" (default) | "left" | "top" | "bottom"
 *   title     ReactNode — rendered in the header row with the close button
 *   size      panel breadth (width for left/right, height for top/bottom)
 *
 * Sub-components: DrawerBody (scrollable), DrawerFooter (action row).
 * `Sheet` is exported as an alias for teams coming from shadcn.
 */
import React, { type ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../icons/icon";
import { IconButton } from "../components/icon-button";
import { useUiText } from "../theme/ui-text";

export type DrawerSide = "left" | "right" | "top" | "bottom";

const SIDE_CLASSES: Record<DrawerSide, string> = {
  right: "inset-y-0 right-0 h-full border-l",
  left: "inset-y-0 left-0 h-full border-r",
  top: "inset-x-0 top-0 w-full border-b",
  bottom: "inset-x-0 bottom-0 w-full border-t",
};

const SIZE_CLASSES = {
  sm: { horizontal: "w-full max-w-xs", vertical: "max-h-[40dvh]" },
  md: { horizontal: "w-full max-w-md", vertical: "max-h-[55dvh]" },
  lg: { horizontal: "w-full max-w-xl", vertical: "max-h-[75dvh]" },
};

export type DrawerSize = keyof typeof SIZE_CLASSES;

type DrawerProps = {
  open: boolean;
  onClose?: () => void;
  side?: DrawerSide;
  size?: DrawerSize;
  title?: ReactNode;
  /** Close-button aria-label; defaults to the UiTextProvider string. */
  closeLabel?: string;
  className?: string;
  children?: ReactNode;
};

export function Drawer({
  open,
  onClose,
  side = "right",
  size = "md",
  title,
  closeLabel,
  className,
  children,
}: DrawerProps) {
  const uiText = useUiText();

  React.useEffect(() => {
    if (!open) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [open]);

  React.useEffect(() => {
    if (!open) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose?.();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!open) return null;

  const axis = side === "left" || side === "right" ? "horizontal" : "vertical";
  const sizeClasses = SIZE_CLASSES[size] ?? SIZE_CLASSES.md;

  return (
    <div className="fixed inset-0 z-50">
      <div
        className="absolute inset-0 bg-black/55 backdrop-blur-sm"
        onClick={onClose}
        aria-hidden="true"
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={typeof title === "string" ? title : undefined}
        className={cn(
          "absolute flex flex-col overflow-hidden",
          "border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)]",
          "shadow-[0_24px_60px_rgba(0,0,0,0.35)]",
          SIDE_CLASSES[side],
          axis === "horizontal" ? sizeClasses.horizontal : sizeClasses.vertical,
          className
        )}
      >
        <div className="flex shrink-0 items-center justify-between gap-4 border-b border-[var(--v2-panel-border)] px-5 py-4">
          <h2 className="text-[1.05rem] font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)]">
            {title}
          </h2>
          {onClose && (
            <IconButton
              variant="outline"
              onClick={onClose}
              aria-label={closeLabel || uiText.close}
            >
              <Icon name="close" className="h-4 w-4" />
            </IconButton>
          )}
        </div>
        {children}
      </div>
    </div>
  );
}

type DrawerSectionProps = {
  children?: ReactNode;
  className?: string;
};

export function DrawerBody({ children, className }: DrawerSectionProps) {
  return (
    <div className={cn("flex-1 overflow-y-auto px-5 py-4 text-ui text-[var(--v2-text)]", className)}>
      {children}
    </div>
  );
}

export function DrawerFooter({ children, className }: DrawerSectionProps) {
  return (
    <div
      className={cn(
        "flex shrink-0 flex-wrap items-center justify-end gap-3 border-t border-[var(--v2-panel-border)] px-5 py-4",
        className
      )}
    >
      {children}
    </div>
  );
}

/** shadcn-familiar alias. */
export const Sheet = Drawer;
export const SheetBody = DrawerBody;
export const SheetFooter = DrawerFooter;
