import { React, html } from "../../lib/html.js";
import { ApprovalCard } from "./components/approval-card.js";
import { ChatInput } from "./components/chat-input.js";
import { ConnectionStatus } from "./components/connection-status.js";
import { EmptyState } from "./components/empty-state.js";
import { MessageList } from "./components/message-list.js";
import { RecoveryNotice } from "./components/recovery-notice.js";
import { SuggestionChips } from "./components/suggestion-chips.js";
import { TypingIndicator } from "./components/typing-indicator.js";
import { useChat } from "./hooks/useChat.js";
import { buildRuntimeContext } from "./lib/runtime-context.js";

export function Chat({
  threads,
  activeThreadId,
  onSelectThread,
  isCreatingThread,
  composerDraft = "",
  composerResetKey = "",
  gatewayStatus,
}) {
  const {
    messages,
    isProcessing,
    pendingGate,
    suggestions,
    sseStatus,
    historyLoading,
    hasMore,
    cooldownSeconds,
    recoveryNotice,
    send,
    retryMessage,
    approve,
    recoverHistory,
    loadMore,
    setSuggestions,
  } = useChat(activeThreadId);

  const activeThread = React.useMemo(
    () => threads.find((thread) => thread.id === activeThreadId) || null,
    [threads, activeThreadId]
  );
  const runtimeContext = React.useMemo(
    () => buildRuntimeContext({ gatewayStatus, activeThread }),
    [gatewayStatus, activeThread]
  );
  const hasMessages =
    messages.length > 0 || isProcessing || Boolean(pendingGate);
  const showLanding = !historyLoading && !hasMessages;
  const composerDisabled = (isProcessing && !pendingGate) || cooldownSeconds > 0;
  const composerStatusText =
    cooldownSeconds > 0 ? `Retry in ${cooldownSeconds}s` : undefined;

  const handleSend = React.useCallback(
    async (content, { images = [], attachments = [] } = {}) => {
      const response = await send(content, {
        images,
        attachments,
        threadId: activeThreadId,
      });
      const responseThreadId = response?.thread_id || activeThreadId;
      if (!activeThreadId && responseThreadId && onSelectThread) {
        onSelectThread(responseThreadId, { replace: true });
      }
      return response;
    },
    [activeThreadId, onSelectThread, send]
  );

  const handleSuggestion = React.useCallback(
    async (text) => {
      setSuggestions([]);
      await handleSend(text);
    },
    [handleSend, setSuggestions]
  );

  return html`
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        <${ConnectionStatus} status=${sseStatus} />

        ${showLanding &&
        html`
          <${EmptyState}
            onSuggestion=${handleSuggestion}
            onSend=${handleSend}
            disabled=${composerDisabled}
            initialText=${composerDraft}
            resetKey=${composerResetKey}
            context=${runtimeContext}
            statusText=${composerStatusText}
          />
        `}
        ${!showLanding &&
        html`
          <${MessageList}
            messages=${messages}
            isLoading=${historyLoading}
            hasMore=${hasMore}
            onLoadMore=${loadMore}
            onRetryMessage=${retryMessage}
          >
            ${recoveryNotice &&
            html`
              <${RecoveryNotice}
                notice=${recoveryNotice}
                onRecover=${recoverHistory}
              />
            `}
            ${isProcessing && !pendingGate && html`<${TypingIndicator} />`}
            ${pendingGate &&
            html`
              <${ApprovalCard}
                gate=${pendingGate}
                onApprove=${() =>
                  approve(pendingGate.requestId, "approve", pendingGate.kind)}
                onDeny=${() =>
                  approve(pendingGate.requestId, "deny", pendingGate.kind)}
                onAlways=${() =>
                  approve(pendingGate.requestId, "always", pendingGate.kind)}
              />
            `}
          <//>

          <${SuggestionChips}
            suggestions=${suggestions}
            onSelect=${handleSuggestion}
          />

          <${ChatInput}
            onSend=${handleSend}
            disabled=${composerDisabled}
            initialText=${composerDraft}
            resetKey=${composerResetKey}
            context=${runtimeContext}
            statusText=${composerStatusText}
          />
        `}
      </div>
    </div>
  `;
}
