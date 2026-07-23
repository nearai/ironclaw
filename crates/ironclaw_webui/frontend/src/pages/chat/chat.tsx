// @ts-nocheck
import React from "react";
import { useT } from "../../lib/i18n";
import {
  THREAD_STATE,
  clearThreadState,
  setThreadState,
} from "../../lib/thread-state";
import { ApprovalCard } from "./components/approval-card";
import { AuthGenericCard } from "./components/auth-generic-card";
import { AuthOauthCard } from "./components/auth-oauth-card";
import { AuthTokenCard } from "./components/auth-token-card";
import { ChatInput } from "./components/chat-input";
import { EmptyState } from "./components/empty-state";
import { KeyboardShortcuts } from "./components/keyboard-shortcuts";
import { MessageList } from "./components/message-list";
import { OnboardingPairingCard } from "./components/onboarding-pairing-card";
import { RecoveryNotice } from "./components/recovery-notice";
import { SuggestionChips } from "./components/suggestion-chips";
import { TypingIndicator } from "./components/typing-indicator";
import { useChat } from "./hooks/useChat";
import { channelConnectionDisplayName } from "../../lib/channel-connection-events";
import { channelConnectionFromGate } from "./lib/gates";
import { NEW_DRAFT_KEY } from "./lib/draft-store";
import { buildRuntimeContext } from "./lib/runtime-context";
import { buildScopedLogsPath } from "../logs/lib/logs-data";
import { useInterfacePreferences } from "../../lib/interface-preferences";

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

function pendingOnboardingLabel(onboarding) {
  // Single source of channel display names (lib/channel-connection-events.ts) so
  // the composer notice and the pairing-card title can't drift in casing.
  return channelConnectionDisplayName(onboarding?.extensionName);
}

function hasVisibleStreamingAssistantText(messages, activeRunId) {
  return (messages || []).some((message) =>
    message?.role === "assistant" &&
    message.isFinalReply === false &&
    typeof message.content === "string" &&
    message.content.length > 0 &&
    (!activeRunId || message.turnRunId === activeRunId)
  );
}

export function Chat({
  threads,
  activeThreadId,
  onSelectThread,
  isCreatingThread,
  composerDraft = "",
  composerResetKey = "",
  gatewayStatus,
  globalAutoApproveEnabled = false,
  onConnectionStatusChange,
}) {
  const t = useT();
  const { showChatLogsShortcut } = useInterfacePreferences();
  const {
    messages,
    isProcessing,
    pendingGate,
    pendingOnboarding,
    busyGateNotice,
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
    startOnboardingOAuth,
    dismissOnboardingPairing,
  } = useChat(activeThreadId);

  React.useEffect(() => {
    onConnectionStatusChange?.(sseStatus);
  }, [onConnectionStatusChange, sseStatus]);

  const activeThread = React.useMemo(
    () => threads.find((thread) => thread.id === activeThreadId) || null,
    [threads, activeThreadId]
  );
  const runtimeContext = React.useMemo(
    () => buildRuntimeContext({ gatewayStatus, activeThread }),
    [gatewayStatus, activeThread]
  );
  const activeThreadHasGate = Boolean(activeThreadId) && Boolean(pendingGate);
  // A channel connection gate is a host-issued PAIRING gate that carries the
  // manifest-derived `connection` context (provider names never select
  // presentation). Deriving it through the shared `channelConnectionFromGate`
  // predicate keeps the composer affordance below and the pairing-card selector
  // (further down) keyed off the SAME condition — a `manual_token` gate can
  // never be shown the token-paste card while the composer promises pairing.
  // Web-generated pairing completes externally through the rendered
  // deep-link/QR flow.
  const channelConnectionGate = channelConnectionFromGate(pendingGate);
  // Normalize the gate's connection context onto the onboarding-shaped prop the
  // pairing card renders from, so one card component serves both entry points.
  const gateConnectionOnboarding = channelConnectionGate
    ? {
        extensionName: channelConnectionGate.channel,
        strategy: channelConnectionGate.strategy,
        instructions: channelConnectionGate.instructions,
        inputPlaceholder: channelConnectionGate.inputPlaceholder,
        submitLabel: channelConnectionGate.submitLabel,
        errorMessage: channelConnectionGate.errorMessage,
      }
    : null;
  const activeThreadHasChannelConnectionGate =
    activeThreadHasGate && Boolean(channelConnectionGate);
  const activeThreadHasOnboarding =
    Boolean(activeThreadId) && Boolean(pendingOnboarding);
  const activeThreadIsProcessing = Boolean(activeThreadId) && isProcessing;
  const activeRunId = activeRun?.runId || null;
  const streamingAssistantTextVisible = hasVisibleStreamingAssistantText(
    messages,
    activeRunId
  );
  const showTypingIndicator =
    activeThreadIsProcessing &&
    !activeThreadHasGate &&
    !streamingAssistantTextVisible;
  const hasMessages =
    messages.length > 0 ||
    activeThreadIsProcessing ||
    activeThreadHasGate ||
    activeThreadHasOnboarding;
  // Don't show the landing composer when history failed to load — show the
  // error banner instead so the user is not misled into thinking the thread
  // is empty.
  const showLanding = !historyLoading && !hasMessages && !historyLoadError;
  const approvalSubmitWarning = activeThreadHasChannelConnectionGate
    ? t("chat.finishPairingBeforeSend")
    : activeThreadHasGate
      ? t("chat.resolveApprovalBeforeSend")
      : activeThreadHasOnboarding
        ? t("chat.finishPairingBeforeSend", {
            name: pendingOnboardingLabel(pendingOnboarding),
          })
        : "";
  const composerSendDisabled =
    activeThreadHasGate ||
    activeThreadHasOnboarding ||
    (activeThreadIsProcessing &&
      !activeThreadHasGate &&
      !activeThreadHasOnboarding) ||
    cooldownSeconds > 0;
  const composerSendBlockedRef = React.useRef(composerSendDisabled);
  composerSendBlockedRef.current = composerSendDisabled;
  const composerStatusText =
    approvalSubmitWarning ||
    (cooldownSeconds > 0 ? t("chat.retryIn", { seconds: cooldownSeconds }) : undefined);
  // Scope the persisted composer draft to the open thread (or the
  // shared new-conversation slot when there's no active thread yet).
  const composerDraftKey = activeThreadId || NEW_DRAFT_KEY;
  const logsPath =
    activeThreadId && showChatLogsShortcut
      ? buildScopedLogsPath({ threadId: activeThreadId })
      : null;
  const canCancelRun = Boolean(
    activeThreadId &&
      activeRun?.runId &&
      activeRun.threadId === activeThreadId &&
      activeThreadIsProcessing &&
      !activeThreadHasGate &&
      !activeThreadHasOnboarding
  );
  const handleSend = React.useCallback(
    async (content, { images = [], attachments = [], displayContent } = {}) => {
      if (activeThreadHasGate) {
        throw new Error(approvalSubmitWarning);
      }
      if (composerSendBlockedRef.current) return null;
      const response = await send(content, {
        images,
        attachments,
        displayContent,
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
      activeThreadHasGate,
      approvalSubmitWarning,
      composerSendDisabled,
      onSelectThread,
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
   *   pendingGate / pendingOnboarding → NEEDS_ATTENTION (amber)
   *   isProcessing without either     → RUNNING (green)
   *   neither                       → clear (idle)
   *
   * Priority is user-action-first because a gate or pairing panel logically
   * subsumes processing — the run is paused waiting on the user, not actively
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
    if (pendingGate || pendingOnboarding) {
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
  }, [activeThreadId, pendingGate, pendingOnboarding, isProcessing]);

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

  return (
    <div className="flex h-full min-h-0 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col">
        {historyLoadError &&
        (
          <div
            className="mx-4 mt-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
            role="alert"
          >
            {t(historyLoadError)}
          </div>
        )}

        {showLanding &&
        (
          <EmptyState
            onSuggestion={handleSuggestion}
            onSend={handleSend}
            disabled={false}
            sendDisabled={composerSendDisabled}
            initialText={composerDraft}
            resetKey={composerResetKey}
            draftKey={composerDraftKey}
            context={runtimeContext}
            statusText={composerStatusText}
            canCancel={canCancelRun}
            onCancel={handleCancelRun}
          />
        )}
        {!showLanding &&
        (
          <>
          <MessageList
            messages={messages}
            isLoading={historyLoading}
            hasMore={hasMore}
            onLoadMore={loadMore}
            onRetryMessage={retryMessage}
            threadId={activeThreadId}
            logsPath={logsPath}
            pending={activeThreadIsProcessing}
          >
            {recoveryNotice &&
            (
              <RecoveryNotice
                notice={recoveryNotice}
                onRecover={recoverHistory}
              />
            )}
            {showTypingIndicator &&
            (<TypingIndicator />)}
            {activeThreadHasOnboarding &&
            (
              <OnboardingPairingCard
                onboarding={pendingOnboarding}
                onConfigure={
                  pendingOnboarding?.strategy === "oauth"
                    ? startOnboardingOAuth
                    : undefined
                }
                onCancel={dismissOnboardingPairing}
              />
            )}
            {pendingGate &&
            (pendingGate.kind === "auth_required"
              ? (pendingGate.challengeKind === "oauth_url"
                ? (
                  <AuthOauthCard
                    gate={pendingGate}
                    onCancel={() =>
                      approve(pendingGate.requestId, "cancel", pendingGate.kind)}
                  />
                )
                : pendingGate.challengeKind === "manual_token"
                  ? (
                  <AuthTokenCard
                    gate={pendingGate}
                    onSubmit={submitAuthToken}
                    onCancel={() =>
                      approve(pendingGate.requestId, "cancel", pendingGate.kind)}
                  />
                )
                  : channelConnectionGate
                  ? (
                  // Same predicate as the composer affordance
                  // (`channelConnectionGate`): a pairing gate carrying manifest
                  // connection context. External completion uses the same
                  // manifest-derived panel as the Extensions surface — there is
                  // nothing to submit to IronClaw; the provider-side action
                  // resumes the run.
                  <OnboardingPairingCard
                    onboarding={gateConnectionOnboarding}
                    onCancel={handleCancelRun}
                  />
                )
                  : (
                  <AuthGenericCard
                    gate={pendingGate}
                    onCancel={() =>
                      approve(pendingGate.requestId, "cancel", pendingGate.kind)}
                  />
                ))
              : (
              <ApprovalCard
                gate={pendingGate}
                globalAutoApproveEnabled={globalAutoApproveEnabled}
                onApprove={() =>
                  approve(pendingGate.requestId, "approve", pendingGate.kind)}
                onDeny={() =>
                  approve(pendingGate.requestId, "deny", pendingGate.kind)}
                onAlways={() =>
                  approve(pendingGate.requestId, "always", pendingGate.kind)}
              />
            ))}
            {busyGateNotice &&
            (
              <div
                data-testid="busy-gate-notice"
                role="status"
                className="mx-auto mt-3 max-w-lg rounded-lg border border-copper/25 bg-copper/10 px-4 py-3 text-center text-sm leading-6 text-copper"
              >
                {busyGateNotice.content}
              </div>
            )}
          </MessageList>

          <SuggestionChips
            suggestions={suggestions}
            onSelect={handleSuggestion}
            disabled={composerSendDisabled}
          />

          <ChatInput
            onSend={handleSend}
            disabled={false}
            sendDisabled={composerSendDisabled}
            initialText={composerDraft}
            resetKey={composerResetKey}
            draftKey={composerDraftKey}
            context={runtimeContext}
            statusText={composerStatusText}
            canCancel={canCancelRun}
            onCancel={handleCancelRun}
          />
          </>
        )}
      </div>
      <KeyboardShortcuts
        open={shortcutsOpen}
        onClose={() => setShortcutsOpen(false)}
      />
    </div>
  );
}
