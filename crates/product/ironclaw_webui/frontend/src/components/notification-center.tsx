import { useNavigate } from "react-router";
import { createPortal } from "react-dom";
import { Button } from "../design-system/button";
import { Icon } from "../design-system/icons";
import React from "react";
import { useT } from "../lib/i18n";
import { cn } from "../utils/cn";

function NotificationRow({ message, unread, onOpen, onArchive }) {
  const t = useT();
  // The row and the archive control are siblings, never nested buttons: a
  // button inside a button is invalid and would make the archive click also
  // open the thread.
  return (
    <div className="flex items-start gap-1 border-b border-[var(--v2-panel-border)] px-4 py-3 last:border-0">
      <button
        type="button"
        disabled={!message.href}
        onClick={message.href ? () => onOpen(message) : undefined}
        data-testid="notification-row"
        className={cn(
          "grid min-w-0 flex-1 grid-cols-[2rem_minmax(0,1fr)] gap-3 rounded-[8px] text-left",
          message.href
            ? "hover:bg-[var(--v2-surface-soft)]"
            : "cursor-default opacity-80"
        )}
      >
      <span
        className="grid h-8 w-8 place-items-center rounded-[8px] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
      >
        <Icon name={message.icon || "bell"} className="h-4 w-4" />
      </span>
      <span className="min-w-0">
        <span className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
            {message.title}
          </span>
          {unread &&
          (<span
            aria-label={t("notifications.unread")}
            className="h-2 w-2 shrink-0 rounded-full bg-[var(--v2-danger-text)]"
          />)}
        </span>
        <span className="mt-0.5 block truncate text-sm text-[var(--v2-text)]">
          {message.body}
        </span>
        <span className="mt-1 flex min-w-0 items-center gap-2 text-[11px] text-[var(--v2-text-faint)]">
          {message.detail &&
          (<span className="truncate">{message.detail}</span>)}
          {message.detail && message.timeLabel &&
          (<span aria-hidden="true">·</span>)}
          {message.timeLabel &&
          (<span className="shrink-0">{message.timeLabel}</span>)}
        </span>
      </span>
      </button>
      {message.durable && onArchive &&
      (<button
        type="button"
        data-testid="notification-archive"
        onClick={() => onArchive(message.id)}
        aria-label={t("common.dismiss")}
        title={t("common.dismiss")}
        className={cn(
          "grid h-7 w-7 shrink-0 place-items-center rounded-[8px]",
          "text-[var(--v2-text-faint)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        )}
      >
        <Icon name="check" className="h-4 w-4" />
      </button>)}
    </div>
  );
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
  const prepareMessageOpen = state?.prepareMessageOpen;
  const markAllRead = state?.markAllRead;
  const archiveMessage = state?.archiveMessage;
  const canLoadMore = state?.canLoadMore || false;
  const loadMore = state?.loadMore;
  const isMarkingAllRead = state?.isMarkingAllRead || false;
  const isLoading = state?.isLoading || false;
  const error = state?.error || null;
  const refetch = state?.refetch;

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
      if (message?.id) {
        if (prepareMessageOpen) prepareMessageOpen(message);
        else dismissMessage?.(message.id);
      }
      close();
      if (message?.href) navigate(message.href);
    },
    [close, dismissMessage, navigate, prepareMessageOpen],
  );

  const overlay = open
    ? (
        <React.Fragment>
          <button
            type="button"
            aria-label={t("notifications.close")}
            onClick={close}
            tabIndex={-1}
            className="fixed inset-0 z-[9998] bg-black/35 lg:bg-transparent"
          />
          <section
            role="dialog"
            aria-label={t("notifications.title")}
            data-testid="notification-panel"
            ref={panelRef}
            tabIndex={-1}
            className={cn(
              "fixed inset-x-0 bottom-0 z-[9999] max-h-[78dvh] overflow-hidden",
              "rounded-t-[16px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] shadow-[0_24px_70px_-24px_rgba(0,0,0,0.8)]",
              "lg:inset-auto lg:right-12 lg:top-16 lg:w-[24rem] lg:max-h-[min(70vh,32rem)] lg:rounded-[12px]"
            )}
          >
            <div className="flex items-center justify-between gap-3 border-b border-[var(--v2-panel-border)] px-4 py-3">
              <div className="min-w-0">
                <h2 className="text-sm font-semibold text-[var(--v2-text-strong)]">
                  {t("notifications.title")}
                </h2>
                <p className="mt-0.5 text-xs text-[var(--v2-text-muted)]">
                  {unreadCount > 0
                    ? t("notifications.unreadCount", { count: unreadCount })
                    : t("notifications.allCaughtUp")}
                </p>
              </div>
              <div className="flex items-center gap-1">
                {unreadCount > 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={markAllRead}
                    disabled={isMarkingAllRead}
                  >
                    {t("notifications.markAllRead")}
                  </Button>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  onClick={close}
                  aria-label={t("notifications.close")}
                  title={t("notifications.close")}
                >
                  <Icon name="close" className="h-4 w-4" />
                </Button>
              </div>
            </div>

            <div className="max-h-[calc(78dvh-4.5rem)] overflow-y-auto lg:max-h-[calc(min(70vh,32rem)-4.5rem)]">
              {isLoading && messages.length === 0
                ? (
                    <div className="px-4 py-8 text-center" role="status">
                      <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                        {t("notifications.loadingTitle")}
                      </div>
                    </div>
                  )
                : error && messages.length === 0
                  ? (
                      <div className="px-4 py-8 text-center" role="alert">
                        <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                          {t("notifications.errorTitle")}
                        </div>
                        <div className="mt-1 text-sm text-[var(--v2-text-muted)]">
                          {t("notifications.errorDescription")}
                        </div>
                        <Button
                          type="button"
                          variant="secondary"
                          size="sm"
                          className="mt-3"
                          onClick={() => refetch?.()}
                        >
                          {t("notifications.retry")}
                        </Button>
                      </div>
                    )
                  : messages.length === 0
                ? (
                    <div className="px-4 py-8 text-center">
                      <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                        {t("notifications.emptyTitle")}
                      </div>
                      <div className="mt-1 text-sm text-[var(--v2-text-muted)]">
                        {t("notifications.emptyDescription")}
                      </div>
                    </div>
                  )
                : messages.map((message) => (
                    <NotificationRow
                      key={message.id}
                      message={message}
                      unread={unreadIds.has(message.id)}
                      onOpen={openMessage}
                      onArchive={archiveMessage}
                    />
                  ))}
              {messages.length > 0 && canLoadMore && loadMore &&
              (<button
                type="button"
                data-testid="notification-load-more"
                onClick={() => loadMore()}
                className={cn(
                  "w-full border-t border-[var(--v2-panel-border)] px-4 py-2.5 text-sm",
                  "text-[var(--v2-accent-text)] hover:bg-[var(--v2-surface-soft)]"
                )}
              >
                {t("common.loadMore")}
              </button>)}
            </div>
          </section>
        </React.Fragment>
      )
    : null;

  return (
    <div className="relative">
      <button
        type="button"
        onClick={toggleOpen}
        data-testid="notification-bell"
        ref={triggerRef}
        aria-label={t("notifications.open")}
        aria-expanded={open ? "true" : "false"}
        className={cn(
          "relative grid h-8 w-8 place-items-center rounded-[8px]",
          "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
          open && "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
        )}
        title={t("notifications.open")}
      >
        <Icon name="bell" className="h-4 w-4" />
        {hasUnread &&
        (
          <span
            data-testid="notification-unread-dot"
            className="absolute right-1.5 top-1.5 h-2.5 w-2.5 rounded-full border-2 border-[var(--v2-canvas-strong)] bg-[var(--v2-danger-text)]"
          />
        )}
      </button>

      {overlay && typeof document !== "undefined"
        ? createPortal(overlay, document.body)
        : null}
    </div>
  );
}
