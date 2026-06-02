import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { MessageBubble } from "./message-bubble.js";
import { ToolRun } from "./tool-activity.js";

/* Collapse consecutive tool-activity messages into runs so a long burst of
   tool calls can render as a single summary line (see ToolRun). Non-tool
   messages pass through unchanged. */
function groupMessages(messages) {
  const items = [];
  let run = null;
  for (const msg of messages) {
    const isToolActivity =
      msg.role === "tool_activity" && !(msg.toolCalls && msg.toolCalls.length > 0);
    if (isToolActivity) {
      if (!run) {
        run = { type: "tool-run", id: `tool-run-${msg.id}`, tools: [] };
        items.push(run);
      }
      run.tools.push(msg);
    } else {
      run = null;
      items.push({ type: "message", id: msg.id, message: msg });
    }
  }
  return items;
}

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
        ${groupMessages(messages).map((item) =>
          item.type === "tool-run"
            ? html`<${ToolRun} key=${item.id} tools=${item.tools} />`
            : html`<${MessageBubble}
                key=${item.id}
                message=${item.message}
                onRetry=${onRetryMessage}
              />`
        )}
        ${children}
      </div>
    </div>
  `;
}
