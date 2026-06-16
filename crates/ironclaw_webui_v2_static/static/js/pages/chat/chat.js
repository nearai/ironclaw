import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
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
import { NEW_DRAFT_KEY } from "./lib/draft-store.js";
import { buildRuntimeContext } from "./lib/runtime-context.js";
import { buildScopedLogsPath } from "../logs/lib/logs-data.js";

export function Chat({
  threads,
  activeThreadId,
  onSelectThread,
  isCreatingThread,
  composerDraft = "",
  composerResetKey = "",
  gatewayStatus,
}) {
  const t = useT();
  const {
    messages,
    isProcessing,
    pendingGate,
    channelConnectAction,
    suggestions,
    sseStatus,
    historyLoading,
    historyLoadError,
    hasMore,
    cooldownSeconds,
    recoveryNotice,
    activeRun,
    send,
    cancelRun,
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
  // Don't show the landing composer when history failed to load — show the
  // error banner instead so the user is not misled into thinking the thread
  // is empty.
  const showLanding = !historyLoading && !hasMessages && !historyLoadError;
  const composerDisabled = (isProcessing && !pendingGate) || cooldownSeconds > 0;
  const composerStatusText =
    cooldownSeconds > 0 ? `Retry in ${cooldownSeconds}s` : undefined;
  // Scope the persisted composer draft to the open thread (or the
  // shared new-conversation slot when there's no active thread yet).
  const composerDraftKey = activeThreadId || NEW_DRAFT_KEY;
  const canCancelRun = Boolean(
    activeThreadId &&
      activeRun?.runId &&
      activeRun.threadId === activeThreadId &&
      isProcessing &&
      !pendingGate
  );
  const scopedLogsHref = React.useMemo(() => {
    if (!activeThreadId) return null;
    const runId =
      activeRun?.threadId === activeThreadId ? activeRun.runId : null;
    return buildScopedLogsPath(
      { threadId: activeThreadId, runId },
      { absolute: true }
    );
  }, [activeRun, activeThreadId]);

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

  const handleCancelRun = React.useCallback(
    () => cancelRun("user_requested"),
    [cancelRun]
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
   * fresh value) on threadId change via the thread-reset effect in
   * useChat, so within a single React commit batch we never observe
   * stale state from a previous thread paired with a new activeThreadId.
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

        ${scopedLogsHref &&
        html`
          <div className="flex justify-end border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-1.5">
            <a
              href=${scopedLogsHref}
              className="rounded-[6px] px-2 py-1 text-xs font-medium text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
            >
              ${t("nav.logs")}
            </a>
          </div>
        `}

        ${historyLoadError &&
        html`
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            ${historyLoadError}
          </div>
        `}

        ${showLanding &&
        html`
          <${EmptyState}
            onSuggestion=${handleSuggestion}
            onSend=${handleSend}
            disabled=${composerDisabled}
            initialText=${composerDraft}
            resetKey=${composerResetKey}
            draftKey=${composerDraftKey}
            context=${runtimeContext}
            statusText=${composerStatusText}
            canCancel=${canCancelRun}
            onCancel=${handleCancelRun}
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
            threadId=${activeThreadId}
            pending=${isProcessing}
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
            draftKey=${composerDraftKey}
            context=${runtimeContext}
            statusText=${composerStatusText}
            canCancel=${canCancelRun}
            onCancel=${handleCancelRun}
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
