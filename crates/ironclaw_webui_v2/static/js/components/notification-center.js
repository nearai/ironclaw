import { useNavigate } from "react-router";
import { createPortal } from "react-dom";
import { Button } from "../design-system/button.js";
import { Icon } from "../design-system/icons.js";
import { React, html } from "../lib/html.js";
import { useT } from "../lib/i18n.js";
import { cn } from "../utils/cn.js";

function NotificationRow({ message, unread, onOpen }) {
  const t = useT();
  return html`
    <button
      type="button"
      disabled=${!message.href}
      onClick=${message.href ? () => onOpen(message) : undefined}
      data-testid="notification-row"
      className=${cn(
        "grid w-full grid-cols-[2rem_minmax(0,1fr)] gap-3 border-b border-[var(--v2-panel-border)] px-4 py-3 text-left last:border-0",
        message.href
          ? "hover:bg-[var(--v2-surface-soft)]"
          : "cursor-default opacity-80"
      )}
    >
      <span
        className="grid h-8 w-8 place-items-center rounded-[8px] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
      >
        <${Icon} name=${message.icon || "bell"} className="h-4 w-4" />
      </span>
      <span className="min-w-0">
        <span className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
            ${message.title}
          </span>
          ${unread &&
          html`<span
            aria-label=${t("notifications.unread")}
            className="h-2 w-2 shrink-0 rounded-full bg-[var(--v2-accent)]"
          />`}
        </span>
        <span className="mt-0.5 block truncate text-sm text-[var(--v2-text)]">
          ${message.body}
        </span>
        <span className="mt-1 flex min-w-0 items-center gap-2 text-[11px] text-[var(--v2-text-faint)]">
          ${message.detail &&
          html`<span className="truncate">${message.detail}</span>`}
          ${message.detail && message.timeLabel &&
          html`<span aria-hidden="true">·</span>`}
          ${message.timeLabel &&
          html`<span className="shrink-0">${message.timeLabel}</span>`}
        </span>
      </span>
    </button>
  `;
}

export function NotificationCenter({ state }) {
  const t = useT();
  const navigate = useNavigate();
  const [open, setOpen] = React.useState(false);
  const panelRef = React.useRef(null);
  const triggerRef = React.useRef(null);
  const messages = state?.messages || [];
  const unreadIds = state?.unreadIds || new Set();
  const hasUnread = state?.hasUnread || false;
  const unreadCount = state?.unreadCount || 0;
  const dismissMessage = state?.dismissMessage;

  const close = React.useCallback(() => {
    setOpen(false);
    triggerRef.current?.focus?.();
  }, []);

  const toggleOpen = React.useCallback(() => {
    const nextOpen = !open;
    setOpen(nextOpen);
    if (!nextOpen) {
      triggerRef.current?.focus?.();
    }
  }, [open]);

  React.useEffect(() => {
    if (!open) return;
    panelRef.current?.focus?.();
  }, [open]);

  React.useEffect(() => {
    if (!open || typeof document === "undefined") return;
    const onKeyDown = (event) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      close();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [close, open]);

  const openMessage = React.useCallback(
    (message) => {
      if (message?.id) dismissMessage?.(message.id);
      close();
      if (message?.href) navigate(message.href);
    },
    [close, dismissMessage, navigate],
  );

  const overlay = open
    ? html`
        <${React.Fragment}>
          <button
            type="button"
            aria-label=${t("notifications.close")}
            onClick=${close}
            tabIndex="-1"
            className="fixed inset-0 z-[9998] bg-black/35 lg:bg-transparent"
          />
          <section
            role="dialog"
            aria-label=${t("notifications.title")}
            data-testid="notification-panel"
            ref=${panelRef}
            tabIndex="-1"
            className=${cn(
              "fixed inset-x-0 bottom-0 z-[9999] max-h-[78dvh] overflow-hidden",
              "rounded-t-[16px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_24px_70px_-24px_rgba(0,0,0,0.8)]",
              "lg:inset-auto lg:right-12 lg:top-16 lg:w-[24rem] lg:max-h-[min(70vh,32rem)] lg:rounded-[12px]"
            )}
          >
            <div className="flex items-center justify-between gap-3 border-b border-[var(--v2-panel-border)] px-4 py-3">
              <div className="min-w-0">
                <h2 className="text-sm font-semibold text-[var(--v2-text-strong)]">
                  ${t("notifications.title")}
                </h2>
                <p className="mt-0.5 text-xs text-[var(--v2-text-muted)]">
                  ${unreadCount > 0
                    ? t("notifications.unreadCount", { count: unreadCount })
                    : t("notifications.allCaughtUp")}
                </p>
              </div>
              <${Button}
                type="button"
                variant="ghost"
                size="icon-sm"
                onClick=${close}
                aria-label=${t("notifications.close")}
                title=${t("notifications.close")}
              >
                <${Icon} name="close" className="h-4 w-4" />
              <//>
            </div>

            <div className="max-h-[calc(78dvh-4.5rem)] overflow-y-auto lg:max-h-[calc(min(70vh,32rem)-4.5rem)]">
              ${messages.length === 0
                ? html`
                    <div className="px-4 py-8 text-center">
                      <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                        ${t("notifications.emptyTitle")}
                      </div>
                      <div className="mt-1 text-sm text-[var(--v2-text-muted)]">
                        ${t("notifications.emptyDescription")}
                      </div>
                    </div>
                  `
                : messages.map((message) => html`
                    <${NotificationRow}
                      key=${message.id}
                      message=${message}
                      unread=${unreadIds.has(message.id)}
                      onOpen=${openMessage}
                    />
                  `)}
            </div>
          </section>
        <//>
      `
    : null;

  return html`
    <div className="relative">
      <button
        type="button"
        onClick=${toggleOpen}
        data-testid="notification-bell"
        ref=${triggerRef}
        aria-label=${t("notifications.open")}
        aria-expanded=${open ? "true" : "false"}
        className=${cn(
          "relative grid h-8 w-8 place-items-center rounded-[8px]",
          "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
          open && "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
        )}
        title=${t("notifications.open")}
      >
        <${Icon} name="bell" className="h-4 w-4" />
        ${hasUnread &&
        html`
          <span
            data-testid="notification-unread-dot"
            className="absolute right-1.5 top-1.5 h-2.5 w-2.5 rounded-full border-2 border-[var(--v2-canvas-strong)] bg-[var(--v2-danger-text)]"
          />
        `}
      </button>

      ${overlay && typeof document !== "undefined"
        ? createPortal(overlay, document.body)
        : null}
    </div>
  `;
}
