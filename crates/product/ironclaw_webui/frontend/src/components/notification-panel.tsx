/* The opened notification panel. It is split out of `notification-center.tsx`
 * and loaded with `React.lazy` from there: the bell and its unread dot are
 * always on screen and must stay eager, but this markup only renders once the
 * reader opens the panel, so it does not belong in the /chat entry closure.
 * Same split the command palette already takes in `layout/gateway-layout.tsx`. */
import { createPortal } from "react-dom";
import { Button } from "../design-system/button";
import { Icon } from "../design-system/icons";
import React from "react";
import { useT } from "../lib/i18n";
import { cn } from "../utils/cn";

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled]):not([tabindex='-1'])",
  "input:not([disabled]):not([tabindex='-1'])",
  "select:not([disabled]):not([tabindex='-1'])",
  "textarea:not([disabled]):not([tabindex='-1'])",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

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
      {onArchive &&
      (<button
        type="button"
        data-testid="notification-archive"
        onClick={() => onArchive(message.id)}
        aria-label={t("notifications.archive")}
        title={t("notifications.archive")}
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

export function NotificationPanel({
  messages,
  unreadIds,
  unreadCount,
  isLoading,
  error,
  refetch,
  markAllRead,
  isMarkingAllRead,
  canLoadMore,
  loadMore,
  pageLimitReached,
  openMessage,
  archiveMessage,
  close,
  panelRef,
}) {
  const t = useT();
  /* Focus lands here on mount, not in the opener: the centre's `[open]` effect
   * runs while `Suspense` is still rendering null, so the ref it reaches for
   * does not exist yet. */
  React.useEffect(() => {
    panelRef?.current?.focus?.();
  }, [panelRef]);
  /* Escape is not handled here: the centre already owns a document-level
   * listener for it, scoped to the open panel, which fires wherever focus sits.
   * A second handler on the dialog would just close it twice. */
  const containFocus = React.useCallback((event) => {
    if (event.key !== "Tab") return;
    const panel = panelRef?.current;
    if (!panel) return;
    const focusable = [...panel.querySelectorAll(FOCUSABLE_SELECTOR)];
    if (focusable.length === 0) {
      event.preventDefault();
      panel.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (!panel.contains(active)) {
      event.preventDefault();
      (event.shiftKey ? last : first).focus();
    } else if (event.shiftKey && (active === first || active === panel)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || active === panel)) {
      event.preventDefault();
      first.focus();
    }
  }, [panelRef]);
  if (typeof document === "undefined") return null;
  return createPortal(
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
            aria-modal="true"
            aria-label={t("notifications.title")}
            data-testid="notification-panel"
            ref={panelRef}
            tabIndex={-1}
            onKeyDown={containFocus}
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
                    data-testid="notification-mark-all-read"
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
              {/* The full-panel error state below only renders on an empty list,
               * so a refresh or a mark-read/archive that fails while rows are on
               * screen used to leave no trace at all — the row simply snapped
               * back from its optimistic update. This banner is that trace. */}
              {error && messages.length > 0 && (
                <div
                  role="alert"
                  data-testid="notification-error-banner"
                  className="flex items-center justify-between gap-2 border-b border-[var(--v2-border)] bg-[var(--v2-surface-muted)] px-4 py-2"
                >
                  <span className="text-sm text-[var(--v2-text-muted)]">
                    {t("notifications.errorTitle")}
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => refetch?.()}
                  >
                    {t("notifications.retry")}
                  </Button>
                </div>
              )}
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
              {pageLimitReached && (
                <div
                  data-testid="notification-page-limit"
                  className="border-t border-[var(--v2-panel-border)] px-4 py-2.5 text-center text-xs text-[var(--v2-text-muted)]"
                >
                  {t("notifications.pageLimit")}
                </div>
              )}
            </div>
          </section>
        </React.Fragment>,
    document.body,
  );
}
