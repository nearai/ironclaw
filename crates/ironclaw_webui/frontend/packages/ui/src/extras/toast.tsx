/**
 * Toast + Toaster
 *
 * Transient notifications built on @radix-ui/react-toast.
 *
 * Two ways to use it:
 *   1. Compositional — render ToastProvider / Toast / ToastTitle /
 *      ToastDescription / ToastAction / ToastClose / ToastViewport yourself
 *      for full control.
 *   2. Imperative — mount a single <Toaster /> host near the app root and
 *      call toast({ title, description, tone }) from anywhere. A module-level
 *      store keeps the API dependency-free (no context import needed at call
 *      sites), mirroring shadcn's useToast pattern.
 *
 * Tones map to the semantic v2 tokens (default / positive / warning / danger).
 */
import * as ToastPrimitive from "@radix-ui/react-toast";
import React, { type ComponentProps, type ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../primitives/icon";

/* ── Compositional pieces ──────────────────────────────────────────── */

export const ToastProvider = ToastPrimitive.Provider;

export function ToastViewport({
  className,
  ...props
}: ComponentProps<typeof ToastPrimitive.Viewport>) {
  return (
    <ToastPrimitive.Viewport
      className={cn(
        "fixed bottom-0 right-0 z-[60] flex w-full max-w-sm list-none flex-col gap-2 p-4 outline-none",
        className
      )}
      {...props}
    />
  );
}

export type ToastTone = "default" | "positive" | "warning" | "danger";

const TONE_BORDER: Record<ToastTone, string> = {
  default: "border-[var(--v2-panel-border)]",
  positive: "border-[color-mix(in_srgb,var(--v2-positive-text)_45%,var(--v2-panel-border))]",
  warning: "border-[color-mix(in_srgb,var(--v2-warning-text)_45%,var(--v2-panel-border))]",
  danger: "border-[color-mix(in_srgb,var(--v2-danger-text)_45%,var(--v2-panel-border))]",
};

const TONE_ICON: Record<ToastTone, ReactNode> = {
  default: null,
  positive: <Icon name="check" className="mt-0.5 h-4 w-4 shrink-0 text-[var(--v2-positive-text)]" />,
  warning: <Icon name="flag" className="mt-0.5 h-4 w-4 shrink-0 text-[var(--v2-warning-text)]" />,
  danger: <Icon name="close" className="mt-0.5 h-4 w-4 shrink-0 text-[var(--v2-danger-text)]" />,
};

type ToastProps = ComponentProps<typeof ToastPrimitive.Root> & {
  tone?: ToastTone;
};

export function Toast({ className, tone = "default", children, ...props }: ToastProps) {
  return (
    <ToastPrimitive.Root
      className={cn(
        "pointer-events-auto relative flex w-full items-start gap-3 overflow-hidden rounded-[12px] border p-4",
        "bg-[var(--v2-card-bg)] text-[var(--v2-text)]",
        "shadow-[0_18px_44px_-16px_rgba(0,0,0,0.55)]",
        "transition-opacity data-[state=closed]:opacity-0",
        TONE_BORDER[tone],
        className
      )}
      {...props}
    >
      {TONE_ICON[tone]}
      {children}
    </ToastPrimitive.Root>
  );
}

export function ToastTitle({
  className,
  ...props
}: ComponentProps<typeof ToastPrimitive.Title>) {
  return (
    <ToastPrimitive.Title
      className={cn("text-ui font-semibold text-[var(--v2-text-strong)]", className)}
      {...props}
    />
  );
}

export function ToastDescription({
  className,
  ...props
}: ComponentProps<typeof ToastPrimitive.Description>) {
  return (
    <ToastPrimitive.Description
      className={cn("text-ui-sm text-[var(--v2-text-muted)]", className)}
      {...props}
    />
  );
}

export function ToastAction({
  className,
  ...props
}: ComponentProps<typeof ToastPrimitive.Action>) {
  return (
    <ToastPrimitive.Action
      className={cn(
        "ml-auto shrink-0 rounded-[8px] border border-[var(--v2-panel-border)] px-2.5 py-1 text-ui-sm font-medium",
        "bg-[var(--v2-surface-soft)] text-[var(--v2-text-strong)] transition-colors",
        "hover:bg-[var(--v2-surface-muted)]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[var(--v2-focus-ring)]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className
      )}
      {...props}
    />
  );
}

export function ToastClose({
  className,
  ...props
}: ComponentProps<typeof ToastPrimitive.Close>) {
  return (
    <ToastPrimitive.Close
      className={cn(
        "absolute right-2 top-2 grid h-6 w-6 place-items-center rounded-[6px]",
        "text-[var(--v2-text-faint)] transition-colors",
        "hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]",
        "active:bg-[var(--v2-surface-muted)]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[var(--v2-focus-ring)]",
        className
      )}
      aria-label="Dismiss"
      {...props}
    >
      <Icon name="close" className="h-3.5 w-3.5" />
    </ToastPrimitive.Close>
  );
}

/* ── Imperative toast() + Toaster host ─────────────────────────────── */

export type ToastOptions = {
  title: ReactNode;
  description?: ReactNode;
  tone?: ToastTone;
  /** Auto-dismiss delay in ms; defaults to the provider's 5000. */
  duration?: number;
};

type ToastRecord = ToastOptions & { id: number; open: boolean };

type ToastListener = (toasts: ToastRecord[]) => void;

let toastSeq = 0;
let toastState: ToastRecord[] = [];
const toastListeners = new Set<ToastListener>();

function emitToasts(next: ToastRecord[]) {
  toastState = next;
  for (const listener of toastListeners) listener(toastState);
}

/**
 * Fire a toast from anywhere. Requires a mounted <Toaster />.
 * Returns the toast id (usable with dismissToast).
 */
export function toast(options: ToastOptions): number {
  toastSeq += 1;
  const record: ToastRecord = { ...options, id: toastSeq, open: true };
  emitToasts([...toastState.slice(-4), record]);
  return record.id;
}

/** Programmatically dismiss one toast (or all when no id is given). */
export function dismissToast(id?: number) {
  emitToasts(
    toastState.map((item) =>
      id === undefined || item.id === id ? { ...item, open: false } : item
    )
  );
}

function useToastStore(): ToastRecord[] {
  return React.useSyncExternalStore(
    (onStoreChange) => {
      toastListeners.add(onStoreChange);
      return () => toastListeners.delete(onStoreChange);
    },
    () => toastState,
    () => toastState
  );
}

type ToasterProps = {
  /** Viewport aria-label announced to screen readers. */
  label?: string;
  className?: string;
};

/** Singleton host for the imperative toast() API. Mount once near the root. */
export function Toaster({ label = "Notifications", className }: ToasterProps) {
  const toasts = useToastStore();
  return (
    <ToastPrimitive.Provider label={label}>
      {toasts.map((item) => (
        <Toast
          key={item.id}
          tone={item.tone ?? "default"}
          duration={item.duration}
          open={item.open}
          onOpenChange={(open) => {
            if (!open) dismissToast(item.id);
          }}
        >
          <div className="flex min-w-0 flex-col gap-0.5 pr-5">
            <ToastTitle>{item.title}</ToastTitle>
            {item.description ? (
              <ToastDescription>{item.description}</ToastDescription>
            ) : null}
          </div>
          <ToastClose />
        </Toast>
      ))}
      <ToastViewport className={className} />
    </ToastPrimitive.Provider>
  );
}
