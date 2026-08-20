import { useNavigate } from "react-router";
import { Icon } from "../design-system/icons";
import React from "react";
import { useT } from "../lib/i18n";
import { cn } from "../utils/cn";

/* The bell and its unread dot are always on screen, so they stay eager; the
 * panel behind them only renders once opened and loads on demand, keeping its
 * markup out of the /chat entry closure. */
const NotificationPanel = React.lazy(() =>
  import("./notification-panel").then(({ NotificationPanel }) => ({
    default: NotificationPanel,
  }))
);

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
  const pageLimitReached = state?.pageLimitReached || false;
  const isMarkingAllRead = state?.isMarkingAllRead || false;
  const isLoading = state?.isLoading || false;
  const error = state?.error || null;
  const refetch = state?.refetch;

  const collapsePages = state?.collapsePages;

  const close = React.useCallback(() => {
    setOpen(false);
    collapsePages?.();
    triggerRef.current?.focus?.();
  }, [collapsePages]);

  const toggleOpen = React.useCallback(() => {
    const nextOpen = !open;
    setOpen(nextOpen);
    if (!nextOpen) {
      collapsePages?.();
      triggerRef.current?.focus?.();
    }
  }, [collapsePages, open]);

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

      {open &&
      (
        <React.Suspense fallback={null}>
          <NotificationPanel
            messages={messages}
            unreadIds={unreadIds}
            unreadCount={unreadCount}
            isLoading={isLoading}
            error={error}
            refetch={refetch}
            markAllRead={markAllRead}
            isMarkingAllRead={isMarkingAllRead}
            canLoadMore={canLoadMore}
            loadMore={loadMore}
            pageLimitReached={pageLimitReached}
            openMessage={openMessage}
            archiveMessage={archiveMessage}
            close={close}
            panelRef={panelRef}
          />
        </React.Suspense>
      )}
    </div>
  );
}
