import { React, html } from "../../lib/html.js";
import { Link } from "react-router";
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

/* Grace window before an active thread's sidebar state is cleared to idle.
 * Long enough for SSE to rehydrate a gate/run after a thread switch (so a
 * persisted "needs attention" badge isn't wiped-then-restored), short
 * enough that a genuinely resolved thread clears promptly.
 *
 * Assumption: SSE rehydration of a live gate/run completes within this
 * window. If it doesn't, a still-pending thread's badge clears here and
 * reappears when the gate finally arrives — a one-off re-flicker, never a
 * wrong state. The downside is purely cosmetic and self-correcting, so it
 * is intentionally not instrumented; revisit this constant (not add
 * telemetry) if slow links make the re-flicker noticeable. */
const THREAD_STATE_CLEAR_GRACE_MS = 1500;

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
    busyGateNotice,
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
  const approvalSubmitWarning = pendingGate
    ? "Resolve the approval request before sending another message."
    : "";
  const composerSendDisabled =
    Boolean(pendingGate) || (isProcessing && !pendingGate) || cooldownSeconds > 0;
  const composerStatusText =
    approvalSubmitWarning ||
    (cooldownSeconds > 0 ? `Retry in ${cooldownSeconds}s` : undefined);
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
  const activeRunLogsPath =
    activeThreadId &&
    activeRun?.runId &&
    activeRun.threadId === activeThreadId
      ? buildScopedLogsPath(
          { threadId: activeThreadId, runId: activeRun.runId },
          { absolute: true },
        )
      : null;
  const handleSend = React.useCallback(
    async (content, { images = [], attachments = [] } = {}) => {
      if (pendingGate) {
        throw new Error(approvalSubmitWarning);
      }
      if (composerSendDisabled) return null;
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
    [
      activeThreadId,
      approvalSubmitWarning,
      composerSendDisabled,
      onSelectThread,
      pendingGate,
      send,
    ]
  );

  const handleSuggestion = React.useCallback(
    async (text) => {
      if (composerSendDisabled) return;
      setSuggestions([]);
      await handleSend(text);
    },
    [composerSendDisabled, handleSend, setSuggestions]
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
   * docs/webui-v2-followup-picks-02-05.md.
   *
   * Clearing is deferred by a short grace period: opening a thread resets
   * pendingGate to null until SSE rehydrates it, so an immediate clear
   * would wipe a persisted "needs attention" badge and re-set it a beat
   * later — a visible flicker on the sidebar row when you click into the
   * thread. An incoming gate/run cancels the pending clear before it
   * fires; a genuinely resolved thread still clears, just after the
   * window. Setting NEEDS_ATTENTION / RUNNING stays immediate. */
  React.useEffect(() => {
    if (!activeThreadId) return undefined;
    if (pendingGate) {
      setThreadState(activeThreadId, THREAD_STATE.NEEDS_ATTENTION);
      return undefined;
    }
    if (isProcessing) {
      setThreadState(activeThreadId, THREAD_STATE.RUNNING);
      return undefined;
    }
    const timer = setTimeout(
      () => clearThreadState(activeThreadId),
      THREAD_STATE_CLEAR_GRACE_MS
    );
    return () => clearTimeout(timer);
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
            disabled=${false}
            sendDisabled=${composerSendDisabled}
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
            ${isProcessing && !pendingGate && html`
              <div className="flex flex-wrap items-center gap-3">
                <${TypingIndicator} />
                ${activeRunLogsPath && html`
                  <${Link}
                    to=${activeRunLogsPath}
                    className="text-xs font-medium text-signal hover:underline"
                  >
                    ${t("nav.logs")}
                  <//>
                `}
              </div>
            `}
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
            ${busyGateNotice &&
            html`
              <div
                data-testid="busy-gate-notice"
                role="status"
                className="mx-auto mt-3 max-w-lg rounded-lg border border-copper/25 bg-copper/10 px-4 py-3 text-center text-sm leading-6 text-copper"
              >
                ${busyGateNotice.content}
              </div>
            `}
          <//>

          <${SuggestionChips}
            suggestions=${suggestions}
            onSelect=${handleSuggestion}
            disabled=${composerSendDisabled}
          />

          <${ChatInput}
            onSend=${handleSend}
            disabled=${false}
            sendDisabled=${composerSendDisabled}
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
