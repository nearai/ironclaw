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
import { authScope } from "../../lib/auth-scope";
import {
  NEW_DRAFT_KEY,
  setDraft,
  setStagedAttachments,
} from "./lib/draft-store";
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
  const [isSubmittingFirstMessage, setIsSubmittingFirstMessage] =
    React.useState(false);
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
    submitOnboardingPairing,
    submitChannelConnectionPairing,
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
  // A channel-pairing gate is a `manual_token` auth gate that also carries a
  // `connection` requirement (gates.ts normalizes it onto `pendingGate.connection`).
  // Render the pairing card off the live gate — wired to a redeem submit and a
  // run-cancel dismiss — instead of the plain token card. This is the generic
  // proof-code channel-connect path; the durable-timeline `pendingOnboarding`
  // panel is the separate, no-active-gate entry point below.
  const channelConnectionGate =
    pendingGate?.kind === "auth_required" &&
    pendingGate?.challengeKind === "manual_token" &&
    pendingGate?.connection
      ? pendingGate.connection
      : null;
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
  // A Telegram BlockedAuth pairing gate is a channel-connection gate too:
  // the composer must say "finish pairing", not "resolve the approval".
  const telegramPairingGate =
    pendingGate?.kind === "auth_required" &&
    pendingGate?.challengeKind === "pairing" &&
    pendingGate?.provider === "telegram";
  const activeThreadHasChannelConnectionGate =
    activeThreadHasGate && (Boolean(channelConnectionGate) || telegramPairingGate);
  const activeThreadHasOnboarding =
    Boolean(activeThreadId) && Boolean(pendingOnboarding);
  const activeThreadIsProcessing = Boolean(activeThreadId) && isProcessing;
  const newConversationIsProcessing =
    !activeThreadId && isSubmittingFirstMessage;
  const chatIsProcessing =
    activeThreadIsProcessing || newConversationIsProcessing;
  const activeRunId = activeRun?.runId || null;
  const streamingAssistantTextVisible = hasVisibleStreamingAssistantText(
    messages,
    activeRunId
  );
  const showTypingIndicator =
    chatIsProcessing &&
    !activeThreadHasGate &&
    !streamingAssistantTextVisible;
  const hasMessages =
    messages.length > 0 ||
    chatIsProcessing ||
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
    (chatIsProcessing &&
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
  // The pending state replaces the landing input with the dock input. If the
  // first send fails, remount that dock as pending clears so it initializes
  // from the draft store restored by handleSend.
  const composerInstanceKey = newConversationIsProcessing
    ? "first-message-submitting"
    : "chat-composer";
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
    async (
      content,
      {
        images = [],
        attachments = [],
        displayContent,
        preserveExistingDraft = false,
      } = {}
    ) => {
      if (activeThreadHasGate) {
        throw new Error(approvalSubmitWarning);
      }
      if (composerSendBlockedRef.current) return null;
      const isFirstMessage = !activeThreadId;
      const submittedScope = isFirstMessage ? authScope() : null;
      const restoreFirstMessageDraft = () => {
        if (
          !isFirstMessage ||
          preserveExistingDraft ||
          authScope() !== submittedScope
        ) {
          return;
        }
        const submittedText =
          typeof displayContent === "string"
            ? displayContent
            : images.length > 0 || attachments.length > 0
              ? ""
              : content;
        setDraft(NEW_DRAFT_KEY, submittedText);
        setStagedAttachments(NEW_DRAFT_KEY, attachments);
      };
      if (isFirstMessage) setIsSubmittingFirstMessage(true);
      try {
        const response = await send(content, {
          images,
          attachments,
          displayContent,
          threadId: activeThreadId,
        });
        if (response === null) restoreFirstMessageDraft();
        const responseThreadId = response?.thread_id || activeThreadId;
        if (!activeThreadId && responseThreadId && onSelectThread) {
          onSelectThread(responseThreadId, { replace: true });
        }
        return response;
      } catch (error) {
        restoreFirstMessageDraft();
        throw error;
      } finally {
        if (isFirstMessage) setIsSubmittingFirstMessage(false);
      }
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
      // A suggestion is independent of anything staged in the landing
      // composer. Leave that draft untouched if creating the thread fails.
      await handleSend(text, { preserveExistingDraft: true });
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
            pending={chatIsProcessing}
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
                onSubmit={submitOnboardingPairing}
                onConfigure={startOnboardingOAuth}
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
                  ? (channelConnectionGate
                    ? (
                  <OnboardingPairingCard
                    onboarding={gateConnectionOnboarding}
                    onSubmit={submitChannelConnectionPairing}
                    onCancel={handleCancelRun}
                  />
                )
                    : (
                  <AuthTokenCard
                    gate={pendingGate}
                    onSubmit={submitAuthToken}
                    onCancel={() =>
                      approve(pendingGate.requestId, "cancel", pendingGate.kind)}
                  />
                ))
                  : telegramPairingGate
                  ? (
                  // A live BlockedAuth pairing gate renders the same pairing
                  // panel the Extensions card uses (dual-surface parity —
                  // docs/reborn/contracts/telegram-v2.md "The in-chat gate").
                  // Pairing completes over the Telegram webhook and the
                  // continuation fanout resumes the run; there is nothing to
                  // submit here, so only cancel is wired.
                  <OnboardingPairingCard
                    onboarding={{
                      extensionName: pendingGate.provider,
                      strategy: "web_generated_code",
                      instructions: pendingGate.body || pendingGate.headline || "",
                    }}
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
            key={composerInstanceKey}
            onSend={handleSend}
            disabled={newConversationIsProcessing}
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
