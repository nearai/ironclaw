import { useNavigate } from "react-router";
import { Button, Popover, PopoverContent, PopoverTrigger } from "@ironclaw/design-system";
import { Icon } from "@ironclaw/design-system";
import React from "react";
import { useT } from "../lib/i18n";
import { cn } from "@ironclaw/design-system";

function NotificationRow({ message, unread, onOpen }) {
  const t = useT();
  return (
    <button
      type="button"
      disabled={!message.href}
      onClick={message.href ? () => onOpen(message) : undefined}
      data-testid="notification-row"
      className={cn(
        "grid w-full grid-cols-[2rem_minmax(0,1fr)] gap-3 border-b border-[var(--v2-panel-border)] px-4 py-3 text-left last:border-0",
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
          <span className="min-w-0 flex-1 truncate text-sm font-medium text-[var(--v2-text-strong)]">
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
  );
}

export function NotificationCenter({ state }) {
  const t = useT();
  const navigate = useNavigate();
  const [open, setOpen] = React.useState(false);
  const messages = state?.messages || [];
  const unreadIds = state?.unreadIds || new Set();
  const hasUnread = state?.hasUnread || false;
  const unreadCount = state?.unreadCount || 0;
  const dismissMessage = state?.dismissMessage;

  const openMessage = React.useCallback(
    (message) => {
      if (message?.id) dismissMessage?.(message.id);
      setOpen(false);
      if (message?.href) navigate(message.href);
    },
    [dismissMessage, navigate],
  );

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          data-testid="notification-bell"
          aria-label={t("notifications.open")}
          title={t("notifications.open")}
          className={cn(
            "relative grid h-8 w-8 place-items-center rounded-[8px]",
            "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
            open && "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
          )}
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
      </PopoverTrigger>

      <PopoverContent
        align="end"
        sideOffset={8}
        collisionPadding={12}
        aria-label={t("notifications.title")}
        data-testid="notification-panel"
        className="z-[9999] w-[24rem] max-w-[calc(100vw-1.5rem)] p-0 bg-[var(--v2-surface)]"
      >
        <div className="flex items-center justify-between gap-3 border-b border-[var(--v2-panel-border)] px-4 py-3">
          <div className="min-w-0">
            <h2 className="text-sm font-medium text-[var(--v2-text-strong)]">
              {t("notifications.title")}
            </h2>
            <p className="mt-0.5 text-xs text-[var(--v2-text-muted)]">
              {unreadCount > 0
                ? t("notifications.unreadCount", { count: unreadCount })
                : t("notifications.allCaughtUp")}
            </p>
          </div>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            onClick={() => setOpen(false)}
            aria-label={t("notifications.close")}
            title={t("notifications.close")}
          >
            <Icon name="close" className="h-4 w-4" />
          </Button>
        </div>

        <div className="max-h-[min(70vh,32rem)] overflow-y-auto">
          {messages.length === 0
            ? (
                <div className="px-4 py-8 text-center">
                  <div className="text-sm font-medium text-[var(--v2-text-strong)]">
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
                />
              ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}
