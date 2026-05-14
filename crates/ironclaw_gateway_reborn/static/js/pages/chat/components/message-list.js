import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { MessageBubble } from "./message-bubble.js";

export function MessageList({
  messages,
  isLoading,
  hasMore,
  onLoadMore,
  onRetryMessage,
  children,
}) {
  const t = useT();
  const containerRef = React.useRef(null);
  const shouldScrollRef = React.useRef(true);

  React.useEffect(() => {
    const el = containerRef.current;
    if (!el || !shouldScrollRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [messages]);

  const onScroll = React.useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const threshold = 100;
    shouldScrollRef.current =
      el.scrollHeight - el.scrollTop - el.clientHeight < threshold;

    if (hasMore && el.scrollTop < threshold && onLoadMore && !isLoading) {
      onLoadMore();
    }
  }, [hasMore, onLoadMore, isLoading]);

  return html`
    <div
      ref=${containerRef}
      onScroll=${onScroll}
      className="flex flex-1 overflow-y-auto px-4 py-6 sm:px-5 lg:px-8"
    >
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-5">
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
        ${messages.map(
          (msg) => html`<${MessageBubble} key=${msg.id} message=${msg} onRetry=${onRetryMessage} />`
        )}
        ${children}
      </div>
    </div>
  `;
}
