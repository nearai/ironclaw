import React from "react";
import { Link } from "react-router";
import { useT } from "../../../lib/i18n";
import { ActivityRun } from "./activity-run";
import { MessageBubble } from "./message-bubble";
import { Icon } from "../../../design-system/icons";
import { groupMessages } from "../lib/message-groups";

export const BOTTOM_FOLLOW_THRESHOLD_PX = 100;
const TOP_LOAD_THRESHOLD_PX = 100;
// The scroll area already keeps bottom padding for transcript-floating
// controls. Keep them inside that padding instead of adding an in-flow spacer
// after the final message.
const FLOATING_CONTROL_BOTTOM_OFFSET_PX = 16;
const FLOATING_CONTROL_STYLE = { bottom: FLOATING_CONTROL_BOTTOM_OFFSET_PX };
const FLOATING_LOGS_BUTTON_CLASS =
  "group absolute right-5 z-10 hidden size-9 items-center justify-center gap-0 overflow-hidden rounded-full border border-[color-mix(in_srgb,var(--v2-accent)_28%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-surface)_88%,var(--v2-accent)_12%)] text-xs font-semibold text-[var(--v2-text-base)] shadow-[0_14px_34px_-18px_rgba(0,0,0,0.95),0_0_0_1px_rgba(255,255,255,0.04)] backdrop-blur-md transition-all hover:border-[color-mix(in_srgb,var(--v2-accent)_50%,var(--v2-panel-border))] hover:bg-[color-mix(in_srgb,var(--v2-surface-muted)_82%,var(--v2-accent)_18%)] hover:text-[var(--v2-text-strong)] focus:outline-none focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_42%,transparent)] sm:inline-flex";
const JUMP_TO_BOTTOM_BUTTON_CLASS =
  "absolute left-1/2 z-10 inline-flex max-w-[calc(100%-2rem)] -translate-x-1/2 items-center gap-1.5 whitespace-nowrap rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]";

export function distanceFromBottom(el) {
  if (!el) return Number.POSITIVE_INFINITY;
  return el.scrollHeight - el.scrollTop - el.clientHeight;
}

export function isNearBottom(el, threshold = BOTTOM_FOLLOW_THRESHOLD_PX) {
  return distanceFromBottom(el) <= threshold;
}

export function scrollToBottom(el) {
  if (!el) return;
  el.scrollTop = Math.max(0, el.scrollHeight - el.clientHeight);
}

export function messageKey(message) {
  if (!message?.id) return null;
  return `${message.role || ""}:${message.id}`;
}

export function isNewUserMessage(previousKey, message) {
  const key = messageKey(message);
  return Boolean(key && message?.role === "user" && key !== previousKey);
}

function plainTextSelection() {
  if (typeof window === "undefined" || !window.getSelection) return "";
  return String(window.getSelection()?.toString?.() || "");
}

export function MessageList({
  messages,
  isLoading,
  hasMore,
  onLoadMore,
  onRetryMessage,
  threadId,
  logsPath,
  pending = false,
  children,
}) {
  const t = useT();
  const containerRef = React.useRef(null);
  const contentRef = React.useRef(null);
  const shouldScrollRef = React.useRef(true);
  const latestMessageKeyRef = React.useRef(null);
  const rafRef = React.useRef(null);
  const scrollRafRef = React.useRef(null);
  const previousScrollTopRef = React.useRef(0);
  const userScrollIntentRef = React.useRef(false);
  const [atBottom, setAtBottom] = React.useState(true);

  const cancelFollow = React.useCallback(() => {
    if (rafRef.current === null) return;
    window.cancelAnimationFrame(rafRef.current);
    rafRef.current = null;
  }, []);

  const followLatest = React.useCallback((force = false) => {
    const el = containerRef.current;
    if (!el) return;
    if (force) {
      shouldScrollRef.current = true;
      userScrollIntentRef.current = false;
    }
    if (!shouldScrollRef.current) return;

    cancelFollow();
    rafRef.current = window.requestAnimationFrame(() => {
      rafRef.current = null;
      const node = containerRef.current;
      if (!node || (!force && !shouldScrollRef.current)) return;
      scrollToBottom(node);
      previousScrollTopRef.current = node.scrollTop;
      userScrollIntentRef.current = false;
      setAtBottom(true);
    });
  }, [cancelFollow]);

  const cancelScrollSync = React.useCallback(() => {
    if (scrollRafRef.current === null) return;
    window.cancelAnimationFrame(scrollRafRef.current);
    scrollRafRef.current = null;
  }, []);

  // Keep the latest content in view. Re-runs on new messages and when the
  // run state flips — the typing indicator / streamed reply are rendered as
  // children (not in `messages`), so they wouldn't trigger this otherwise.
  // useLayoutEffect avoids painting a newly streamed chunk below the viewport
  // before the follow-scroll runs.
  React.useLayoutEffect(() => {
    const latestMessage = messages.length > 0 ? messages[messages.length - 1] : null;
    const latestKey = messageKey(latestMessage);
    const force = isNewUserMessage(latestMessageKeyRef.current, latestMessage);
    latestMessageKeyRef.current = latestKey;
    followLatest(force);
    return cancelFollow;
  }, [messages, pending, followLatest, cancelFollow]);

  React.useLayoutEffect(() => {
    const target = contentRef.current;
    if (!target || typeof ResizeObserver !== "function") return undefined;
    // Some rendered content grows after message state has committed, such as
    // markdown/code block layout. Keep following those changes without stacking
    // multiple scroll frames.
    const observer = new ResizeObserver(() => {
      followLatest();
    });
    observer.observe(target);
    return () => {
      observer.disconnect();
      cancelFollow();
    };
  }, [followLatest, cancelFollow]);

  const syncScrollState = React.useCallback(() => {
    scrollRafRef.current = null;
    const el = containerRef.current;
    if (!el) return;
    const nearBottom = isNearBottom(el);
    previousScrollTopRef.current = el.scrollTop;
    if (nearBottom) {
      shouldScrollRef.current = true;
      userScrollIntentRef.current = false;
      setAtBottom(true);
    } else if (userScrollIntentRef.current) {
      shouldScrollRef.current = false;
      setAtBottom(false);
    } else {
      shouldScrollRef.current = true;
      setAtBottom(true);
      followLatest();
    }

    if (
      hasMore &&
      el.scrollTop < TOP_LOAD_THRESHOLD_PX &&
      onLoadMore &&
      !isLoading
    ) {
      onLoadMore();
    }
  }, [hasMore, onLoadMore, isLoading, followLatest]);

  const markUserScrollIntent = React.useCallback(() => {
    userScrollIntentRef.current = true;
  }, []);

  const markScrollbarDragIntent = React.useCallback((event) => {
    const el = containerRef.current;
    if (!el || typeof event?.clientX !== "number") return;
    const scrollbarWidth = el.offsetWidth - el.clientWidth;
    if (scrollbarWidth <= 0) return;
    const rightEdge = el.getBoundingClientRect().right;
    if (event.clientX >= rightEdge - scrollbarWidth - 2) {
      userScrollIntentRef.current = true;
    }
  }, []);

  const onScroll = React.useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const nearBottom = isNearBottom(el);
    const isUpwardScroll = el.scrollTop < previousScrollTopRef.current;
    previousScrollTopRef.current = el.scrollTop;
    if (!nearBottom && isUpwardScroll) {
      userScrollIntentRef.current = true;
    }
    if (nearBottom) {
      shouldScrollRef.current = true;
      userScrollIntentRef.current = false;
    } else if (userScrollIntentRef.current) {
      shouldScrollRef.current = false;
      cancelFollow();
    } else {
      shouldScrollRef.current = true;
    }
    if (scrollRafRef.current !== null) return;
    scrollRafRef.current = window.requestAnimationFrame(syncScrollState);
  }, [cancelFollow, syncScrollState]);

  const jumpToBottom = React.useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    scrollToBottom(el);
    previousScrollTopRef.current = el.scrollTop;
    shouldScrollRef.current = true;
    userScrollIntentRef.current = false;
    setAtBottom(true);
  }, []);

  const onCopy = React.useCallback((event) => {
    const text = plainTextSelection();
    if (!text || !event.clipboardData) return;
    event.preventDefault();
    event.clipboardData.clearData();
    event.clipboardData.setData("text/plain", text);
  }, []);

  React.useEffect(() => cancelScrollSync, [cancelScrollSync]);

  const grouped = React.useMemo(() => groupMessages(messages), [messages]);

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 overflow-hidden">
    <div
      ref={containerRef}
      onScroll={onScroll}
      onWheel={markUserScrollIntent}
      onTouchMove={markUserScrollIntent}
      onPointerDown={markScrollbarDragIntent}
      onCopy={onCopy}
      data-testid="message-list-scroll"
      className="flex min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-3 pt-5 pb-14 sm:px-5 sm:pt-6 lg:px-8"
    >
      <div
        ref={contentRef}
        data-testid="message-list-content"
        className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-4 sm:gap-5"
      >
        {hasMore &&
        (
          <div className="text-center">
            <button
              onClick={onLoadMore}
              disabled={isLoading}
              data-testid="message-list-load-older"
              className="v2-button rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-300 hover:border-signal/35 hover:text-white disabled:opacity-50"
            >
              {isLoading
                ? t("chat.history.loading")
                : t("chat.history.loadOlder")}
            </button>
          </div>
        )}
        {grouped.map((item) =>
          item.type === "activity-run"
            ? (<ActivityRun key={item.id} activity={item.activity} />)
            : (<MessageBubble
                key={item.id}
                message={item.message}
                onRetry={onRetryMessage}
                threadId={threadId}
              />)
        )}
        {children}
      </div>
    </div>
    {logsPath &&
    (
      <Link
        to={logsPath}
        aria-label={t("nav.logs")}
        title={t("nav.logs")}
        className={FLOATING_LOGS_BUTTON_CLASS}
        style={FLOATING_CONTROL_STYLE}
      >
        <Icon name="logs" className="size-5" />
      </Link>
    )}
    {!atBottom &&
    (
      <button
        type="button"
        onClick={jumpToBottom}
        aria-label={t("chat.jumpToLatest")}
        className={JUMP_TO_BOTTOM_BUTTON_CLASS}
        style={FLOATING_CONTROL_STYLE}
      >
        <Icon name="arrowDown" className="h-3.5 w-3.5" />
        {t("chat.jumpToLatest")}
      </button>
    )}
    </div>
  );
}
