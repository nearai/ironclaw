import { React, html } from "../../lib/html.js";
import {
  THREAD_STATE,
  clearThreadState,
  setThreadState,
} from "../../lib/thread-state.js";
import { ApprovalCard } from "./components/approval-card.js";
import { AuthGenericCard } from "./components/auth-generic-card.js";
import { AuthOauthCard } from "./components/auth-oauth-card.js";
import { AuthTokenCard } from "./components/auth-token-card.js";
import { ChannelConnectCard } from "./components/channel-connect-card.js";
import { ChatInput } from "./components/chat-input.js";
import { ConnectionStatus } from "./components/connection-status.js";
import { EmptyState } from "./components/empty-state.js";
import { KeyboardShortcuts } from "./components/keyboard-shortcuts.js";
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
    channelConnectAction,
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
    submitAuthToken,
    dismissChannelConnectAction,
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
    messages.length > 0 || isProcessing || Boolean(pendingGate) || Boolean(channelConnectAction);
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

  /* Mirror the active thread's lifecycle into the per-thread state store
   * so the sidebar row reflects what's happening on the open thread:
   *
   *   pendingGate                   → NEEDS_ATTENTION (amber)
   *   isProcessing && !pendingGate  → RUNNING (green)
   *   neither                       → clear (idle)
   *
   * Priority is pendingGate-first because a gate logically subsumes
   * processing — the run is paused waiting on the user, not actively
   * working.
   *
   * Invariant: useChat resets pendingGate (and isProcessing reaches a
   * fresh value) on threadId change via the sibling effect at
   * useChat.js:136-140, so within a single React commit batch we never
   * observe stale state from a previous thread paired with a new
   * activeThreadId.
   *
   * Coverage gap (writer is per-active-thread only): this seam only
   * flags whichever thread the user is currently viewing. Cross-thread
   * visibility — the green/amber dot appearing on background threads
   * — requires either a user-scoped SSE channel or list_threads state
   * enrichment. Both are deferred follow-ups; see
   * docs/webui-v2-followup-picks-02-05.md. */
  React.useEffect(() => {
    if (!activeThreadId) return;
    if (pendingGate) {
      setThreadState(activeThreadId, THREAD_STATE.NEEDS_ATTENTION);
    } else if (isProcessing) {
      setThreadState(activeThreadId, THREAD_STATE.RUNNING);
    } else {
      clearThreadState(activeThreadId);
    }
  }, [activeThreadId, pendingGate, isProcessing]);

  const [shortcutsOpen, setShortcutsOpen] = React.useState(false);
  React.useEffect(() => {
    const onKeyDown = (event) => {
      if (event.key === "Escape") {
        setShortcutsOpen(false);
        return;
      }
      if (event.key !== "?") return;
      const target = event.target;
      const tag = target?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || target?.isContentEditable) return;
      event.preventDefault();
      setShortcutsOpen((open) => !open);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

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
            ${channelConnectAction &&
            html`
              <${ChannelConnectCard}
                connectAction=${channelConnectAction}
                onDismiss=${dismissChannelConnectAction}
              />
            `}
            ${pendingGate &&
            (pendingGate.kind === "auth_required"
              ? (pendingGate.challengeKind === "oauth_url"
                ? html`
                  <${AuthOauthCard}
                    gate=${pendingGate}
                    onCancel=${() =>
                      approve(pendingGate.requestId, "cancel", pendingGate.kind)}
                  />
                `
                : pendingGate.challengeKind === "manual_token"
                  ? html`
                  <${AuthTokenCard}
                    gate=${pendingGate}
                    onSubmit=${submitAuthToken}
                    onCancel=${() =>
                      approve(pendingGate.requestId, "cancel", pendingGate.kind)}
                  />
                `
                  : html`
                  <${AuthGenericCard}
                    gate=${pendingGate}
                    onCancel=${() =>
                      approve(pendingGate.requestId, "cancel", pendingGate.kind)}
                  />
                `)
              : html`
              <${ApprovalCard}
                gate=${pendingGate}
                onApprove=${() =>
                  approve(pendingGate.requestId, "approve", pendingGate.kind)}
                onDeny=${() =>
                  approve(pendingGate.requestId, "deny", pendingGate.kind)}
                onAlways=${() =>
                  approve(pendingGate.requestId, "always", pendingGate.kind)}
              />
            `)}
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
      <${KeyboardShortcuts}
        open=${shortcutsOpen}
        onClose=${() => setShortcutsOpen(false)}
      />
    </div>
  `;
}
