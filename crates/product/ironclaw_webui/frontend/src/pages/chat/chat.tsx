// @ts-nocheck
import React from "react";
import { useLocation } from "react-router";
import { useT } from "../../lib/i18n";
import { toast } from "../../lib/toast";
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
import { useChatCommands } from "./hooks/useChatCommands";
import { matchCommand } from "./lib/chat-commands";
import { channelConnectionDisplayName } from "../../lib/channel-connection-events";
import { channelConnectionFromGate, gateIsDeviceLink } from "./lib/gates";
import { NEW_DRAFT_KEY } from "./lib/draft-store";
import { buildRuntimeContext } from "./lib/runtime-context";
import { buildScopedLogsPath } from "../logs/lib/logs-data";
import { useInterfacePreferences } from "../../lib/interface-preferences";
import {
  inspectorDebugEnabled,
  latestInspectorRunId,
  persistInspectorDebugPreference,
} from "./inspector/inspector-shell";

let LazyInspectorPanel: React.LazyExoticComponent<
  React.ComponentType<{ threadId: string | null; runId: string | null }>
> | null = null;

function getInspectorPanel() {
  LazyInspectorPanel ??= React.lazy(() =>
    import("./inspector/inspector-panel").then(({ InspectorPanel }) => ({
      default: InspectorPanel,
    })),
  );
  return LazyInspectorPanel;
}

// The device-link card carries a whole multi-step flow — payload rendering,
// step machine, polling, input forms — for a gate most sessions never see.
// Loaded on demand for the same reason the inspector is: the initial /chat
// route pays for what every chat needs, not for every card it might ever show.
let LazyAuthDeviceLinkCard: React.LazyExoticComponent<
  React.ComponentType<{ gate: unknown; onCancel: () => void }>
> | null = null;

function getAuthDeviceLinkCard() {
  LazyAuthDeviceLinkCard ??= React.lazy(() =>
    import("./components/auth-device-link-card").then(({ AuthDeviceLinkCard }) => ({
      default: AuthDeviceLinkCard,
    })),
  );
  return LazyAuthDeviceLinkCard;
}

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

function cancellationFailureDiagnostic(error) {
  const status =
    error &&
    typeof error === "object" &&
    Number.isInteger(error.status) &&
    error.status >= 400 &&
    error.status <= 599
      ? error.status
      : null;
  return status === null
    ? { category: "request_error" }
    : { category: "http_error", status };
}

export function Chat({
  threads,
  activeThreadId,
  onSelectThread,
  isCreatingThread,
  composerDraft = "",
  composerResetKey = "",
  gatewayStatus,
  regressionArtifactExportEnabled = false,
  globalAutoApproveEnabled = false,
  onConnectionStatusChange,
}) {
  const t = useT();
  const location = useLocation();
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
    runCommand,
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
  const chatCommands = useChatCommands();

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
  // Same one-predicate discipline as the pairing selector above: the branch
  // and the chunk it pulls are decided together, never re-derived at render.
  const DeviceLinkCard = gateIsDeviceLink(pendingGate) ? getAuthDeviceLinkCard() : null;
  const activeThreadHasOnboarding =
    Boolean(activeThreadId) && Boolean(pendingOnboarding);
  const activeThreadIsProcessing = Boolean(activeThreadId) && isProcessing;
  const activeRunId = activeRun?.runId || null;
  const inspectorEnabled = inspectorDebugEnabled(location.search);
  React.useEffect(() => {
    persistInspectorDebugPreference(location.search);
  }, [location.search]);
  const inspectorRunId = React.useMemo(
    () => latestInspectorRunId(activeRun, messages),
    [activeRun, messages],
  );
  const InspectorPanel = inspectorEnabled ? getInspectorPanel() : null;
  const showTypingIndicator =
    activeThreadIsProcessing &&
    !activeThreadHasGate;
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
  // Queued-message UX: a running thread no longer disables the composer — a
  // follow-up sent while a run is active is accepted and queued. Only a
  // pending gate / onboarding step (which needs the user's input first) or an
  // active cooldown blocks a send.
  const composerSendDisabled =
    activeThreadHasGate ||
    activeThreadHasOnboarding ||
    cooldownSeconds > 0;
  const composerSendBlockedRef = React.useRef(composerSendDisabled);
  composerSendBlockedRef.current = composerSendDisabled;
  // Identifies which "empty-thread cycle" may navigate away from the
  // landing view. It's bumped on every truthy->falsy transition of
  // activeThreadId (a genuine new cycle, e.g. "+ New") and by whichever
  // send wins the navigation for the current cycle -- so a captured id
  // only matches while its cycle is still current and unclaimed. That one
  // comparison is enough to stop every stale closure from a batch of
  // concurrent landing-composer sends from re-navigating, whether its
  // cycle was already claimed by an earlier winner or superseded by a
  // "+ New" before it settled. Each redundant navigation tears down and
  // reopens the app's single SSE stream, and those reconnects are
  // genuinely accepted, so they burn the caller's server-side rate-limit
  // budget and strand WebChat on the "Disconnected" badge.
  const previousActiveThreadIdRef = React.useRef(activeThreadId);
  const emptyThreadCycleIdRef = React.useRef(0);
  if (previousActiveThreadIdRef.current && !activeThreadId) {
    emptyThreadCycleIdRef.current += 1;
  }
  previousActiveThreadIdRef.current = activeThreadId;
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
      const sendCycleId = emptyThreadCycleIdRef.current;
      // A response naming a thread other than the selected/active one —
      // a newly created landing thread, or a command effect such as `/new`
      // opening a fresh task — routes the browser to it, exactly as the send
      // path already did, so the result (a system notice for a command, the
      // first reply for a message) renders somewhere visible. From the
      // landing view, only the send that still owns the current empty-thread
      // cycle may navigate; see `emptyThreadCycleIdRef`.
      const selectResponseThread = (response) => {
        const responseThreadId = response?.thread_id || activeThreadId;
        if (!responseThreadId || !onSelectThread) return;
        if (activeThreadId) {
          // `previousActiveThreadIdRef` is reassigned on every render, so
          // between renders it holds the LATEST selection. A command that
          // resolves after the user already opened another thread no longer
          // matches its origin selection and must not steal the newer one
          // (the landing path gets the same protection from the cycle fence
          // below).
          if (
            responseThreadId !== activeThreadId &&
            previousActiveThreadIdRef.current === activeThreadId
          ) {
            onSelectThread(responseThreadId, { replace: true });
          }
          return;
        }
        if (emptyThreadCycleIdRef.current === sendCycleId) {
          emptyThreadCycleIdRef.current += 1;
          onSelectThread(responseThreadId, { replace: true });
        }
      };
      // Slash text naming an inventory command executes as a product command
      // (no turn); anything else — including unknown slash text — submits as
      // an ordinary message, matching channel behavior. Commands require an
      // existing conversation (the execute route is thread-scoped): running
      // one from the landing composer with no thread yet created one and
      // then lost the result to the thread-load race — the new thread's
      // history loads empty and wipes the just-appended notice, leaving an
      // empty conversation behind. Rather than fix that ordering, homepage
      // commands are intentionally disabled for now — do not drop the
      // `activeThreadId` precondition below to "fix" this; the fix is to not
      // offer commands there at all.
      if (
        activeThreadId &&
        images.length === 0 &&
        attachments.length === 0 &&
        matchCommand(content, chatCommands)
      ) {
        const response = await runCommand(content);
        selectResponseThread(response);
        return response;
      }
      const response = await send(content, {
        images,
        attachments,
        displayContent,
        threadId: activeThreadId,
      });
      selectResponseThread(response);
      return response;
    },
    [
      activeThreadId,
      activeThreadHasGate,
      approvalSubmitWarning,
      chatCommands,
      composerSendDisabled,
      onSelectThread,
      runCommand,
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

  // Starting an OOBE suggestion is a server-side operation: the backend creates
  // the suggestion's own thread and submits its turn through the normal
  // ProductSurface path, then returns the binding. The browser only navigates
  // to the thread it is handed — it does not inject the prompt itself.
  const handleOpenSuggestionThread = React.useCallback(
    (threadId) => {
      if (!threadId || !onSelectThread) return;
      onSelectThread(threadId, { replace: true });
    },
    [onSelectThread]
  );

  const handleCancelRun = React.useCallback(
    async () => {
      try {
        await cancelRun("user_requested");
      } catch (error) {
        console.error(
          "Failed to cancel active run",
          cancellationFailureDiagnostic(error)
        );
        toast(t("chat.cancelFailed"), { tone: "error" });
      }
    },
    [cancelRun, t]
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
            onOpenThread={handleOpenSuggestionThread}
            commands={activeThreadId ? chatCommands : []}
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
            activeRunId={activeRunId}
            regressionArtifactExportEnabled={
              regressionArtifactExportEnabled
            }
            logsPath={logsPath}
            pending={activeThreadIsProcessing}
            commands={chatCommands}
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
                : DeviceLinkCard
                  ? (
                  // A device link is a multi-step vendor handshake, not a
                  // credential to paste: the card drives the flow itself and
                  // the run resumes when the host takes custody of the
                  // session. Cancelling abandons the parked turn, exactly as
                  // the pairing card does.
                  <React.Suspense fallback={null}>
                    <DeviceLinkCard gate={pendingGate} onCancel={handleCancelRun} />
                  </React.Suspense>
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
            commands={activeThreadId ? chatCommands : []}
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
      {InspectorPanel && (
        <React.Suspense fallback={null}>
          <InspectorPanel threadId={activeThreadId} runId={inspectorRunId} />
        </React.Suspense>
      )}
    </div>
  );
}
