import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { ActivityRun } from "./activity-run.js";
import { MessageBubble } from "./message-bubble.js";
import { Icon } from "../../../design-system/icons.js";
import { groupMessages } from "../lib/message-groups.js";

export function MessageList({
  messages,
  isLoading,
  hasMore,
  onLoadMore,
  onRetryMessage,
  threadId,
  pending = false,
  children,
}) {
  const t = useT();
  const containerRef = React.useRef(null);
  const shouldScrollRef = React.useRef(true);
  const [atBottom, setAtBottom] = React.useState(true);

  // Keep the latest content in view. Re-runs on new messages and when the
  // run state flips — the typing indicator / streamed reply are rendered as
  // children (not in `messages`), so they wouldn't trigger this otherwise.
  // The rAF defers the scroll until after layout so `scrollHeight` reflects
  // the just-rendered row (markdown, code blocks).
  React.useEffect(() => {
    const el = containerRef.current;
    if (!el || !shouldScrollRef.current) return;
    const raf = window.requestAnimationFrame(() => {
      const node = containerRef.current;
      if (node) node.scrollTop = node.scrollHeight;
    });
    return () => window.cancelAnimationFrame(raf);
  }, [messages, pending]);

  const onScroll = React.useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const threshold = 100;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    shouldScrollRef.current = distance < threshold;
    setAtBottom(distance < threshold);

    if (hasMore && el.scrollTop < threshold && onLoadMore && !isLoading) {
      onLoadMore();
    }
  }, [hasMore, onLoadMore, isLoading]);

  const jumpToBottom = React.useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    shouldScrollRef.current = true;
    setAtBottom(true);
  }, []);

  const grouped = React.useMemo(() => groupMessages(messages), [messages]);

  return html`
    <div className="relative flex min-h-0 min-w-0 flex-1">
    <div
      ref=${containerRef}
      onScroll=${onScroll}
      className="flex min-w-0 flex-1 overflow-y-auto px-4 pt-6 pb-14 sm:px-5 lg:px-8"
    >
      <div className="mx-auto flex w-full min-w-0 max-w-5xl flex-col gap-5">
        ${hasMore &&
        html`
          <div className="text-center">
            <button
              onClick=${onLoadMore}
              disabled=${isLoading}
              className="v2-button rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-300 hover:border-signal/35 hover:text-white disabled:opacity-50"
            >
              ${isLoading
                ? t("chat.history.loading")
                : t("chat.history.loadOlder")}
            </button>
          </div>
        `}
        ${grouped.map((item) =>
          item.type === "activity-run"
            ? html`<${ActivityRun} key=${item.id} activity=${item.activity} />`
            : html`<${MessageBubble}
                key=${item.id}
                message=${item.message}
                onRetry=${onRetryMessage}
                threadId=${threadId}
              />`
        )}
        ${children}
      </div>
    </div>
    ${!atBottom &&
    html`
      <button
        type="button"
        onClick=${jumpToBottom}
        aria-label=${t("chat.jumpToLatest")}
        className="absolute bottom-4 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] shadow-[0_10px_30px_-12px_rgba(0,0,0,0.7)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))]"
      >
        <${Icon} name="arrowDown" className="h-3.5 w-3.5" />
        ${t("chat.jumpToLatest")}
      </button>
    `}
    </div>
  `;
}
