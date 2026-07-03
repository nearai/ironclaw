import {
  cancelRun as cancelRunRequest,
  createThread as createThreadRequest,
  resolveGate as resolveGateRequest,
  sendMessage,
  submitManualToken,
} from "../../../lib/api.js";
import {
  channelConnectionContinuationMessage,
  connectionEventMatchesOnboarding,
  forgetChannelConnectionWaiter,
  normalizeConnectionChannel,
  notifyChannelConnected,
  rememberChannelConnectionWaiter,
  subscribeChannelConnected,
} from "../../../lib/channel-connection-events.js";
import { queryClient } from "../../../lib/query-client.js";
import { React } from "../../../lib/html.js";
import {
  fetchExtensionSetup,
  fetchExtensions,
  startExtensionOauth,
} from "../../extensions/lib/extensions-api.js";
import { redeemPairingCode } from "../../extensions/lib/pairing-api.js";
import { useChatEvents } from "../lib/useChatEvents.js";
import { touchThreadInCache } from "../lib/thread-cache.js";
import {
  addPending,
  recordAcceptedMessageRef,
  removePending,
  timelineMessageIdFromAcceptedRef,
} from "../lib/pending-messages.js";
import {
  createToolActivityState,
  failGateToolActivity,
  resetToolActivityState,
} from "../lib/tool-activity-state.js";
import { toRenderAttachment, toWireAttachment } from "../lib/attachments.js";
import { useHistory } from "./useHistory.js";
import { useSSE } from "./useSSE.js";

const AUTH_TOKEN_FLOW_TIMEOUT_MS = 30000;
const AUTH_GATE_CREDENTIAL_STORED_ERROR =
  "credential_stored_gate_resolution_failed";
const APPROVAL_GATE_PENDING_SEND_ERROR = "approval_gate_pending_send_blocked";
const OAUTH_CALLBACK_CHANNEL = "ironclaw-product-auth";
const OAUTH_CALLBACK_STORAGE_KEY = "ironclaw:product-auth:oauth-complete";
const OAUTH_CALLBACK_MESSAGE_TYPE = "ironclaw:product-auth:oauth-complete";
const DISMISSED_ONBOARDING_STORAGE_PREFIX =
  "ironclaw.chat.dismissedOnboarding.v1:";
const DISMISSED_ONBOARDING_STORAGE_LIMIT = 100;

async function withAuthTokenTimeout(task) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), AUTH_TOKEN_FLOW_TIMEOUT_MS);
  try {
    return await task(controller.signal);
  } finally {
    clearTimeout(timeout);
  }
}

function credentialStoredGateResolutionError(cause) {
  const error = new Error("auth gate resolution failed after credential storage");
  error.safeAuthGateCode = AUTH_GATE_CREDENTIAL_STORED_ERROR;
  error.cause = cause;
  return error;
}

function approvalGatePendingSendError() {
  const error = new Error(
    "Resolve the approval request before sending another message.",
  );
  error.safeErrorCode = APPROVAL_GATE_PENDING_SEND_ERROR;
  return error;
}

function isHttpsAuthUrl(url) {
  try {
    return new URL(url).protocol === "https:";
  } catch (_) {
    return false;
  }
}

function openAuthUrl(url, popup = null) {
  if (!isHttpsAuthUrl(url)) return { ok: false, popup: null };
  if (popup && !popup.closed) {
    popup.location.href = url;
    return { ok: true, popup };
  }
  const opened = window.open(url, "_blank", "noopener,noreferrer");
  return {
    ok: Boolean(opened),
    popup: opened || null,
    reason: opened ? null : "popup_blocked",
  };
}

function threadNeedsSidebarRefresh(threadId) {
  const cached = queryClient.getQueryData?.(["threads"]);
  const threads = cached?.threads;
  if (!Array.isArray(threads)) return true;
  const thread = threads.find((item) => item.thread_id === threadId || item.id === threadId);
  return !thread?.title;
}

function busyNoticeKey(threadId, gate) {
  if (!threadId || !gate?.runId || !gate?.gateRef) return null;
  return `${threadId}\n${gate.runId}\n${gate.gateRef}`;
}

function onboardingBelongsToThread(onboarding, threadId) {
  if (!onboarding) return false;
  const onboardingThreadId = String(onboarding.threadId || "").trim();
  // A pairing panel is per-thread. An onboarding with no thread id belongs to no
  // thread, so it must not leak onto every chat the user opens — the live derive
  // path always stamps the current thread, so this only guards a malformed writer.
  if (!onboardingThreadId) return false;
  const currentThreadId = String(threadId || "").trim();
  return Boolean(currentThreadId) && onboardingThreadId === currentThreadId;
}

function dismissedOnboardingStorageKey(threadId) {
  const normalized = String(threadId || "").trim();
  return normalized ? `${DISMISSED_ONBOARDING_STORAGE_PREFIX}${normalized}` : null;
}

function onboardingStorage() {
  try {
    return globalThis?.localStorage || null;
  } catch (_) {
    return null;
  }
}

function loadDismissedOnboardingIds(threadId) {
  const key = dismissedOnboardingStorageKey(threadId);
  const storage = key ? onboardingStorage() : null;
  if (!storage) return new Set();
  try {
    const parsed = JSON.parse(storage.getItem(key) || "[]");
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((value) => typeof value === "string"));
  } catch (_) {
    return new Set();
  }
}

function persistDismissedOnboardingId(threadId, sourceMessageId) {
  const key = dismissedOnboardingStorageKey(threadId);
  const storage = key ? onboardingStorage() : null;
  if (!storage || !sourceMessageId) return;
  const dismissed = loadDismissedOnboardingIds(threadId);
  dismissed.add(sourceMessageId);
  const values = Array.from(dismissed).slice(-DISMISSED_ONBOARDING_STORAGE_LIMIT);
  try {
    storage.setItem(key, JSON.stringify(values));
  } catch (_) {
    // Best-effort UX preference; storage failures should not block chat.
  }
}

// The backend marks an `extension_activate` result for a connectable channel
// with this `output_kind`; the structured connect action rides on the card's
// `toolResultPreview` JSON. This is the structured replacement for the deleted
// prose regex — the panel is derived from typed fields, never from message text.
const CHANNEL_CONNECTION_REQUIRED_OUTPUT_KIND = "channel_connection_required";

function channelConnectionRequirementFromCard(card) {
  if (!card || card.outputKind !== CHANNEL_CONNECTION_REQUIRED_OUTPUT_KIND) return null;
  if (card.toolStatus && card.toolStatus !== "success") return null;
  if (typeof card.toolResultPreview !== "string" || !card.toolResultPreview.trim()) {
    return null;
  }
  let parsed;
  try {
    parsed = JSON.parse(card.toolResultPreview);
  } catch (_) {
    return null;
  }
  const channel = String(parsed?.channel || "").trim();
  if (!channel) return null;
  return {
    sourceMessageId: card.id || null,
    extensionName: channel,
    strategy: typeof parsed.strategy === "string" ? parsed.strategy : null,
    instructions: typeof parsed.instructions === "string" ? parsed.instructions : null,
    inputPlaceholder:
      typeof parsed.input_placeholder === "string" ? parsed.input_placeholder : null,
    submitLabel: typeof parsed.submit_label === "string" ? parsed.submit_label : null,
    errorMessage: typeof parsed.error_message === "string" ? parsed.error_message : null,
  };
}

// The most recent channel-connection-required card in the timeline, unless the
// user already dismissed it (the dismissal is keyed by the durable tool id, so a
// closed panel does not re-derive from the still-present card on reload).
function latestChannelConnectionRequirement(messages, dismissedIds) {
  if (!Array.isArray(messages)) return null;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const requirement = channelConnectionRequirementFromCard(messages[index]);
    if (!requirement) continue;
    if (requirement.sourceMessageId && dismissedIds?.has?.(requirement.sourceMessageId)) {
      return null;
    }
    if (threadResumedAfterConnection(messages, index, requirement.extensionName)) {
      return null;
    }
    return requirement;
  }
  return null;
}

// A connection card is consumed once the durable timeline already carries the
// channel-connected continuation after it: pairing completed (here, in
// another chat, or in another tab) and this thread was resumed. Unlike the
// localStorage dismissal set — which only records panels dismissed or
// submitted in *this* browser — the continuation is part of the thread's own
// data, so the verdict holds across browsers and after the extension is
// later removed. A fresh requirement on such a thread arrives as a new
// backend card on the next send.
function threadResumedAfterConnection(messages, cardIndex, channel) {
  const expected = channelConnectionContinuationMessage(channel);
  for (let index = cardIndex + 1; index < messages.length; index += 1) {
    const message = messages[index];
    if (message?.role !== "user") continue;
    const content = typeof message?.content === "string" ? message.content : "";
    if (content.trim() === expected) return true;
  }
  return false;
}

// True when the caller's account for `channel` is already connected, per the
// per-user extensions snapshot. The card is gated on this so a durable
// activation card can never re-open the panel for an already-connected account.
function channelConnectionIsSatisfied(extensions, channel) {
  // Normalize both operands the same way the waiter bus does
  // (lib/channel-connection-events.js) so a multi-word channel id (e.g.
  // `telegram_bot`) can't satisfy the gate here while the bus keys on a different
  // normalized string — which would re-open the panel for a connected account.
  const expected = normalizeConnectionChannel(channel);
  const extension = (extensions || []).find(
    (item) =>
      normalizeConnectionChannel(
        item?.package_ref?.id || item?.packageRef?.id || item?.id || "",
      ) === expected,
  );
  if (!extension) return false;
  if (extension.authenticated === true && extension.needs_setup !== true) return true;
  if (extension.authenticated === false || extension.needs_setup === true) return false;
  const state =
    extension.onboarding_state ||
    extension.onboardingState ||
    extension.activation_status ||
    extension.activationStatus;
  // Fail closed: an explicit backend connect card must not be suppressed by a
  // missing or unrecognized onboarding state. Treat the account as connected only
  // when it reports a state that is not a "needs connection" one.
  if (typeof state !== "string" || !state) return false;
  return !["setup_required", "pairing_required", "pairing"].includes(state);
}

function submitResponseResumedTurnGate(response) {
  return response?.continuation?.type === "turn_gate_resume";
}

function resolveGateOutcome(response) {
  if (response?.outcome) return response.outcome;
  const status = String(response?.status || "").toLowerCase();
  if (status === "queued" || status === "running") return "resumed";
  if (status === "cancelled" || response?.already_terminal === true) {
    return "cancelled";
  }
  if (response?.already_terminal === false) return "resumed";
  return null;
}

function isPendingOAuthGate(gate) {
  return gate?.kind === "auth_required" && gate?.challengeKind === "oauth_url";
}

function isOAuthCallbackCompletion(payload) {
  return payload?.type === OAUTH_CALLBACK_MESSAGE_TYPE && payload?.status === "completed";
}

function oauthCompletionMatchesGate(payload, gate, listeningSince) {
  if (!isOAuthCallbackCompletion(payload)) return false;
  const continuation = payload?.continuation;
  if (!continuation || continuation.type !== "turn_gate_resume") {
    return Number(payload?.completedAt || 0) >= listeningSince;
  }
  if (continuation.turn_run_ref && continuation.turn_run_ref !== gate?.runId) return false;
  if (continuation.gate_ref && continuation.gate_ref !== gate?.gateRef) return false;
  return true;
}

function parseOAuthCallbackStoragePayload(value) {
  if (!value) return null;
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

// v2 chat hook. Differences from the fork's v1 hook:
// - No image / attachment plumbing — v2 SendMessage carries `content` only.
// - No /api/chat/approval — approvals fold into gate/resolve in v2.
// - resolveGate uses `runId` + `gateRef` from the live event stream, not
//   a v1-style `requestId`.
// - cancelRun is a first-class action and posts to the v2 cancel route.
export function useChat(threadId) {
  const threadIdRef = React.useRef(threadId);
  const pendingMessagesRef = React.useRef(new Map());
  const pendingSeqRef = React.useRef(1);
  const [cooldownUntil, setCooldownUntil] = React.useState(0);
  const [now, setNow] = React.useState(Date.now());
  const [activeRun, setActiveRunState] = React.useState(null);
  const activeRunRef = React.useRef(activeRun);
  const setActiveRun = React.useCallback((next) => {
    const value = typeof next === "function" ? next(activeRunRef.current) : next;
    activeRunRef.current = value;
    setActiveRunState(value);
  }, []);
  // Mirror committed activeRun into the ref. The setActiveRun wrapper keeps
  // the ref current for back-to-back synchronous reads inside event handlers;
  // this effect additionally covers paths that set the state directly — the
  // per-thread reset below uses the raw setter so render stays side-effect
  // free (no ref mutation during render, which a concurrent render could
  // discard without rolling back).
  React.useEffect(() => {
    activeRunRef.current = activeRun;
  }, [activeRun]);
  const getPendingMessages = React.useCallback(
    () => pendingMessagesRef.current.get(threadId || "__new__") || [],
    [threadId],
  );
  const setPendingMessages = React.useCallback(
    (messages) => {
      const key = threadId || "__new__";
      if (messages.length > 0) {
        pendingMessagesRef.current.set(key, messages);
      } else {
        pendingMessagesRef.current.delete(key);
      }
    },
    [threadId],
  );

  const {
    messages,
    messagesThreadId,
    hasMore,
    nextCursor,
    isLoading: historyLoading,
    loadError: historyLoadError,
    loadHistory,
    seedThreadMessages,
    setMessages,
  } = useHistory(threadId, { getPendingMessages, setPendingMessages });

  const [isProcessing, setIsProcessingState] = React.useState(false);
  const isProcessingRef = React.useRef(isProcessing);
  const setIsProcessing = React.useCallback((next) => {
    const value =
      typeof next === "function" ? next(isProcessingRef.current) : next;
    isProcessingRef.current = value;
    setIsProcessingState(value);
  }, []);
  const [pendingGate, setPendingGateState] = React.useState(null);
  const pendingGateRef = React.useRef(pendingGate);
  const [pendingOnboarding, setPendingOnboardingState] = React.useState(null);
  const pendingOnboardingRef = React.useRef(pendingOnboarding);
  const pendingOnboardingOauthFlowRef = React.useRef(null);
  const sendRef = React.useRef(null);
  // Source tool-message ids whose pairing panel the user dismissed. Keyed by
  // the durable `tool-<invocation_id>`, so a dismissal survives re-renders and
  // timeline reloads and the still-present activation tool-result does not
  // re-derive a panel the user already closed.
  const dismissedOnboardingIdsRef = React.useRef(new Set());
  const [busyGateNotice, setBusyGateNotice] = React.useState(null);
  const setPendingGate = React.useCallback((next) => {
    const current = pendingGateRef.current;
    const value =
      typeof next === "function" ? next(current) : next;
    if (Object.is(value, current)) return;
    pendingGateRef.current = value;
    setPendingGateState(value);
  }, []);
  const setPendingOnboarding = React.useCallback((next) => {
    const current = pendingOnboardingRef.current;
    const value =
      typeof next === "function" ? next(current) : next;
    if (Object.is(value, current)) return;
    pendingOnboardingRef.current = value;
    setPendingOnboardingState(value);
  }, []);
  const [stateThreadId, setStateThreadId] = React.useState(threadId);
  const toolActivityStateRef = React.useRef(createToolActivityState());
  const locallyResolvedGatesRef = React.useRef(new Map());
  const authTokenSubmitRef = React.useRef({
    gateKey: null,
    credentialRef: null,
    inFlight: false,
  });
  const submitBusyRef = React.useRef(false);
  const localRunAdmissionRef = React.useRef(null);

  // Per-thread transient state must not leak across thread switches.
  // Without this reset, clicking "+ New" while the previous thread is
  // still processing renders the TypingIndicator on the empty new
  // thread. The SSE subscription for the new thread will set these
  // back to non-default values if that thread actually has an active
  // run / gate. `cooldownUntil` is intentionally not reset — it's a
  // rate-limit timer that applies across threads.
  //
  // This runs DURING render (not in an effect) on purpose. An effect
  // fires a beat too late: there is one render where the new threadId is
  // already in scope but pendingGate / isProcessing still hold the prior
  // thread's values, and any consumer reading them in that render (the
  // approval card, and the sidebar state mirror in chat.js) briefly
  // mis-attributes the old thread's gate to the newly opened one — e.g.
  // a "needs attention" badge bleeding onto a normal thread. React
  // supports a conditional setState during render for exactly this
  // "adjust state when a prop changes" case; it re-renders immediately
  // without committing the stale output. The previous-threadId guard is
  // itself state (not a ref) so an aborted concurrent render rolls it
  // back and the reset re-fires on retry instead of being skipped.
  //
  // DO NOT move this into a useEffect — that is the regression it fixes.
  // Two rules keep this pattern correct, and any change here must preserve
  // both: (1) the guard must be state, not a ref, so it
  // is rolled back on a discarded render; (2) only plain state setters may
  // run here (no ref writes / side effects) — that is why this uses the
  // raw setActiveRunState rather than the activeRunRef-mutating wrapper.
  if (stateThreadId !== threadId) {
    setStateThreadId(threadId);
    setIsProcessingState(false);
    setPendingGateState(null);
    setPendingOnboardingState(null);
    setBusyGateNotice(null);
    setActiveRunState(null);
  }

  React.useEffect(() => {
    threadIdRef.current = threadId;
  }, [threadId]);
  React.useEffect(
    () => () => {
      if (localRunAdmissionRef.current?.threadId === threadId) {
        localRunAdmissionRef.current = null;
      }
    },
    [threadId],
  );

  React.useEffect(() => {
    pendingGateRef.current = pendingGate;
  }, [pendingGate]);
  React.useEffect(() => {
    pendingOnboardingRef.current = pendingOnboarding;
  }, [pendingOnboarding]);
  React.useEffect(() => {
    isProcessingRef.current = isProcessing;
  }, [isProcessing]);

  React.useEffect(() => {
    const currentKey = busyNoticeKey(threadId, pendingGate);
    setBusyGateNotice((current) =>
      current && current.gateKey !== currentKey ? null : current,
    );
  }, [pendingGate, threadId]);

  React.useEffect(() => {
    resetToolActivityState(toolActivityStateRef);
    locallyResolvedGatesRef.current.clear();
    dismissedOnboardingIdsRef.current = loadDismissedOnboardingIds(threadId);
  }, [threadId]);

  const cooldownSeconds = Math.max(0, Math.ceil((cooldownUntil - now) / 1000));
  const visiblePendingOnboarding = onboardingBelongsToThread(
    pendingOnboarding,
    threadId,
  )
    ? pendingOnboarding
    : null;
  const pendingAuthGateKey =
    pendingGate?.runId && pendingGate?.gateRef
      ? `${pendingGate.runId}\n${pendingGate.gateRef}`
      : null;

  React.useEffect(() => {
    if (!cooldownUntil) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [cooldownUntil]);

  React.useEffect(() => {
    if (authTokenSubmitRef.current.gateKey !== pendingAuthGateKey) {
      authTokenSubmitRef.current = {
        gateKey: pendingAuthGateKey,
        credentialRef: null,
        inFlight: false,
      };
    }
  }, [pendingAuthGateKey]);

  React.useEffect(() => {
    if (pendingOnboarding?.threadId && pendingOnboarding?.extensionName) {
      rememberChannelConnectionWaiter({
        channel: pendingOnboarding.extensionName,
        threadId: pendingOnboarding.threadId,
        sourceMessageId: pendingOnboarding.sourceMessageId || null,
      });
    }
  }, [
    pendingOnboarding?.extensionName,
    pendingOnboarding?.threadId,
    pendingOnboarding?.sourceMessageId,
  ]);

  const clearOnboardingAfterChannelConnected = React.useCallback(
    (onboarding) => {
      const threadForResume = onboarding?.threadId || threadId || null;
      if (!threadForResume || !onboarding?.extensionName) return;
      if (onboarding.sourceMessageId) {
        dismissedOnboardingIdsRef.current.add(onboarding.sourceMessageId);
        persistDismissedOnboardingId(threadForResume, onboarding.sourceMessageId);
      }
      forgetChannelConnectionWaiter({
        channel: onboarding.extensionName,
        threadId: threadForResume,
        sourceMessageId: onboarding.sourceMessageId || null,
      });
      setPendingOnboarding(null);
    },
    [threadId, setPendingOnboarding],
  );

  // Derive the in-chat pairing panel from the durable, structured
  // channel-connection-required tool card the backend attaches to an
  // `extension_activate` result for a connectable channel. The trigger is a
  // concrete per-thread card (present only after the agent engaged the channel
  // here), so — unlike the removed global poll — it never fires on an empty or
  // unrelated chat and never races history loading. It is gated on the live
  // per-user connection state so a card that outlived the user's connect can
  // never re-open the panel for an already-connected account.
  React.useEffect(() => {
    if (!threadId || pendingGate || pendingOnboardingRef.current) return;
    // Only derive from messages that belong to the active thread. On a thread
    // switch `threadId` advances a render before useHistory swaps `messages` to
    // the new thread's timeline; without this guard the previous thread's durable
    // connection card would open — and be stamped onto — the newly-viewed chat.
    if (messagesThreadId && messagesThreadId !== threadId) return;
    const requirement = latestChannelConnectionRequirement(
      messages,
      dismissedOnboardingIdsRef.current,
    );
    if (!requirement) return;
    let cancelled = false;
    Promise.resolve(
      typeof queryClient.fetchQuery === "function"
        ? queryClient.fetchQuery({ queryKey: ["extensions"], queryFn: fetchExtensions })
        : fetchExtensions(),
    )
      .then((data) => {
        if (cancelled || pendingOnboardingRef.current) return;
        if (channelConnectionIsSatisfied(data?.extensions || [], requirement.extensionName)) {
          return;
        }
        setPendingOnboarding({
          extensionName: requirement.extensionName,
          state: "pairing_required",
          threadId,
          sourceMessageId: requirement.sourceMessageId,
          strategy: requirement.strategy,
          instructions: requirement.instructions,
          inputPlaceholder: requirement.inputPlaceholder,
          submitLabel: requirement.submitLabel,
          errorMessage: requirement.errorMessage,
        });
      })
      .catch(() => {
        if (cancelled || pendingOnboardingRef.current) return;
        setPendingOnboarding({
          extensionName: requirement.extensionName,
          state: "pairing_required",
          threadId,
          sourceMessageId: requirement.sourceMessageId,
          strategy: requirement.strategy,
          instructions: requirement.instructions,
          inputPlaceholder: requirement.inputPlaceholder,
          submitLabel: requirement.submitLabel,
          errorMessage: requirement.errorMessage,
        });
      });
    return () => {
      cancelled = true;
    };
  }, [messages, messagesThreadId, threadId, pendingGate, setPendingOnboarding]);

  React.useEffect(() => {
    if (!isPendingOAuthGate(pendingGate)) return;
    const browserWindow =
      typeof window !== "undefined" ? window : globalThis?.window || null;
    if (!browserWindow) return;
    const listeningSince = Date.now();

    const handleCompletion = (payload) => {
      if (!oauthCompletionMatchesGate(payload, pendingGate, listeningSince)) return;
      setPendingGate((current) => (isPendingOAuthGate(current) ? null : current));
      setIsProcessing(true);
    };

    let channel = null;
    if (typeof browserWindow.BroadcastChannel === "function") {
      channel = new browserWindow.BroadcastChannel(OAUTH_CALLBACK_CHANNEL);
      channel.onmessage = (event) => handleCompletion(event.data);
    }

    const onStorage = (event) => {
      if (event.key !== OAUTH_CALLBACK_STORAGE_KEY) return;
      handleCompletion(parseOAuthCallbackStoragePayload(event.newValue));
    };

    browserWindow.addEventListener?.("storage", onStorage);
    handleCompletion(
      parseOAuthCallbackStoragePayload(
        browserWindow.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY),
      ),
    );
    const timer = browserWindow.setInterval(() => {
      handleCompletion(
        parseOAuthCallbackStoragePayload(
          browserWindow.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY),
        ),
      );
    }, 500);
    return () => {
      browserWindow.clearInterval(timer);
      if (channel) channel.close();
      browserWindow.removeEventListener?.("storage", onStorage);
    };
  }, [pendingGate]);

  React.useEffect(() => {
    const browserWindow =
      typeof window !== "undefined" ? window : globalThis?.window || null;
    if (!browserWindow) return;
    let serverCheckInFlight = false;
    const finishCompletion = async (pending) => {
      if (!pending || pending.completing) return;
      pendingOnboardingOauthFlowRef.current = { ...pending, completing: true };
      const onboarding = pendingOnboardingRef.current;
      const threadForResume =
        pending.threadId || onboarding?.threadId || threadId || null;
      let sourceCleared = false;
      if (connectionEventMatchesOnboarding({ channel: pending.channel }, onboarding)) {
        if (threadForResume && !onboarding?.requestId) {
          const sendForResume = sendRef.current;
          if (typeof sendForResume !== "function") {
            pendingOnboardingOauthFlowRef.current = pending;
            return;
          }
          const continuation = await sendForResume(
            channelConnectionContinuationMessage(pending.channel),
            {
              threadId: threadForResume,
              bypassPendingOnboarding: true,
            },
          );
          if (!continuation || continuation.outcome === "rejected_busy") {
            pendingOnboardingOauthFlowRef.current = pending;
            return;
          }
        }
        clearOnboardingAfterChannelConnected(onboarding);
        sourceCleared = true;
      }
      pendingOnboardingOauthFlowRef.current = null;
      await notifyChannelConnected({
        channel: pending.channel,
        sourceThreadId: sourceCleared ? threadForResume : null,
        source: "chat-oauth",
      });
    };
    const handleCompletion = (payload) => {
      const pending = pendingOnboardingOauthFlowRef.current;
      if (!pending) return;
      if (!payload || payload.flowId !== pending.flowId) return;
      if (payload.status !== "completed") return;
      Promise.resolve(finishCompletion(pending)).catch(() => {
        pendingOnboardingOauthFlowRef.current = pending;
      });
    };
    const pollServerState = () => {
      const pending = pendingOnboardingOauthFlowRef.current;
      if (!pending || pending.completing || serverCheckInFlight) return;
      serverCheckInFlight = true;
      Promise.resolve(fetchExtensions())
        .then((snapshot) => {
          const extensions = Array.isArray(snapshot)
            ? snapshot
            : snapshot?.extensions || [];
          if (channelConnectionIsSatisfied(extensions, pending.channel)) {
            return finishCompletion(pending);
          }
          return null;
        })
        .catch(() => null)
        .finally(() => {
          serverCheckInFlight = false;
        });
    };

    let channel = null;
    if (typeof browserWindow.BroadcastChannel === "function") {
      channel = new browserWindow.BroadcastChannel(OAUTH_CALLBACK_CHANNEL);
      channel.onmessage = (event) => handleCompletion(event.data);
    }

    const onStorage = (event) => {
      if (event.key !== OAUTH_CALLBACK_STORAGE_KEY) return;
      handleCompletion(parseOAuthCallbackStoragePayload(event.newValue));
    };

    browserWindow.addEventListener?.("storage", onStorage);
    const timer = browserWindow.setInterval(() => {
      handleCompletion(
        parseOAuthCallbackStoragePayload(
          browserWindow.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY),
        ),
      );
      pollServerState();
    }, 500);
    return () => {
      browserWindow.clearInterval(timer);
      if (channel) channel.close();
      browserWindow.removeEventListener?.("storage", onStorage);
    };
  }, [clearOnboardingAfterChannelConnected, threadId]);

  const handleEvent = useChatEvents({
    threadId,
    setMessages,
    setIsProcessing,
    setPendingGate,
    setActiveRun,
    activeRunRef,
    locallyResolvedGatesRef,
    toolActivityStateRef,
    // Reborn's projection bridge does not yet emit `Text` items for
    // assistant replies, and never emits `capability_display_preview`
    // items in the projection state — the assistant reply and the rich
    // tool input/output cards live only in the thread timeline. Refetch
    // the timeline on EVERY terminal run (success or not) so both become
    // visible; a failed/cancelled run still recovers the tool previews for
    // tools that completed before it terminated. `preserveClientOnly`
    // keeps the client-side `err-*` failure bubble across the reload.
    // On success, clear pending optimistic messages first so the real
    // user message from the server doesn't render alongside its
    // pre-submit optimistic twin.
    onRunSettled: (_runId, { success }) => {
      const localRunAdmission = localRunAdmissionRef.current;
      if (localRunAdmission?.runId === _runId) {
        localRunAdmissionRef.current = null;
      } else if (_runId && localRunAdmission && !localRunAdmission.runId) {
        // The terminal SSE can arrive before the POST response exposes run_id.
        localRunAdmissionRef.current = {
          ...localRunAdmission,
          runId: _runId,
          settledBeforeResponse: true,
        };
      }
      // submitBusyRef is released by send()'s `finally` when the POST settles —
      // it is NOT this callback's to clear. Releasing the POST re-entrancy guard
      // on run settlement is the wrong layer (and was the deadlock #5256 fixed).
      if (success) setPendingMessages([]);
      loadHistory(undefined, {
        preserveClientOnly: true,
        finalReplyTimestampByRun:
          _runId && success ? { [_runId]: new Date().toISOString() } : null,
      });
    },
  });

  const { status: sseStatus } = useSSE({
    threadId,
    onEvent: handleEvent,
    enabled: Boolean(threadId),
  });

  // Accepts the composer call shape `{ attachments, threadId }`. The
  // `attachments` are staged objects from `lib/attachments.js`
  // (`stageFiles`); we split them into the `WebUiInboundAttachment` wire
  // shape for the send and the render shape for the optimistic bubble so
  // cards/thumbnails appear immediately, matching what the timeline
  // projection returns after the run.
  //
  // v2 send-message requires `thread_id` as a path parameter — the
  // facade refuses to implicitly create a missing thread. When the
  // caller is on the landing screen (no active thread yet), we
  // eagerly POST `/threads` first and use the returned id. The
  // returned response carries `thread_id` so the chat.js navigation
  // hook can route to `/chat/<id>` after the first send.
  const send = React.useCallback(
    async (content, opts = {}) => {
      const {
        threadId: targetThreadId,
        attachments: stagedAttachments = [],
        bypassPendingOnboarding = false,
        displayContent,
      } = opts;
      const wireAttachments = stagedAttachments.map(toWireAttachment);
      const renderAttachments = stagedAttachments.map(toRenderAttachment);
      const renderContent =
        typeof displayContent === "string" ? displayContent : content;

      if (
        pendingGate ||
        pendingGateRef.current ||
        (!bypassPendingOnboarding &&
          (onboardingBelongsToThread(pendingOnboarding, targetThreadId || threadId) ||
            onboardingBelongsToThread(
              pendingOnboardingRef.current,
              targetThreadId || threadId,
            )))
      ) {
        throw approvalGatePendingSendError();
      }
      // Admission: block a send only when the *destination* thread is the one
      // that's busy. The destination is `targetThreadId` when the caller names
      // one, otherwise the open thread (the same `targetThreadId || threadId`
      // resolved below). BOTH the in-flight-run guard and the viewed-thread
      // `isProcessing` flag must key on that destination — a running thread
      // carries both, so narrowing only one still drops a parallel send to
      // another thread, or a new chat, just because the thread on screen is
      // running. Keying either guard on the viewed thread (or on the mere
      // absence of a target) is what broke parallel threads and "new chat
      // while a run is active".
      const sendTargetThreadId = targetThreadId || threadId;
      const activeRunForSend = activeRunRef.current;
      const activeRunBlocksSend =
        Boolean(activeRunForSend) &&
        Boolean(sendTargetThreadId) &&
        activeRunForSend.threadId === sendTargetThreadId;
      const processingBlocksSend =
        isProcessingRef.current &&
        Boolean(sendTargetThreadId) &&
        sendTargetThreadId === threadId;
      const localRunBlocksSend =
        Boolean(sendTargetThreadId) &&
        localRunAdmissionRef.current?.threadId === sendTargetThreadId;
      if (
        submitBusyRef.current ||
        processingBlocksSend ||
        activeRunBlocksSend ||
        localRunBlocksSend
      ) {
        return null;
      }

      let sendThreadId = targetThreadId || threadId;

      if (!sendThreadId) {
        const created = await createThreadRequest();
        queryClient.invalidateQueries({ queryKey: ["threads"] });
        sendThreadId = created?.thread?.thread_id;
        if (!sendThreadId) {
          throw new Error("createThread returned no thread_id");
        }
      }

      const pendingKey = sendThreadId;
      const pendingRecord = {
        id: `pending-${pendingSeqRef.current++}`,
        role: "user",
        content: renderContent,
        attachments: renderAttachments,
        retryContent: content,
        retryDisplayContent: renderContent,
        retryAttachments: stagedAttachments,
        timestamp: new Date().toISOString(),
        isOptimistic: true,
      };
      const pendingRenderMessage = {
        id: pendingRecord.id,
        role: "user",
        content: renderContent,
        attachments: renderAttachments,
        retryContent: content,
        retryDisplayContent: renderContent,
        retryAttachments: stagedAttachments,
        timestamp: pendingRecord.timestamp,
        isOptimistic: true,
      };
      addPending(pendingMessagesRef.current, pendingKey, pendingRecord);

      const optimisticId = pendingRecord.id;
      const shouldRenderInCurrentThread = !threadId || sendThreadId === threadId;
      const updateCurrentThread = (updater) => {
        if (shouldRenderInCurrentThread) setMessages(updater);
      };
      const updateSeededTarget = (updater) => {
        if (sendThreadId !== threadId) seedThreadMessages(sendThreadId, updater);
      };
      const updateCurrentRunState = (updater) => {
        if (shouldRenderInCurrentThread) updater();
      };
      // Only the rendered thread has an SSE settle path in this hook. Background
      // target sends are left to the server's rejected_busy response instead.
      const shouldTrackLocalRun = shouldRenderInCurrentThread;
      if (shouldTrackLocalRun) {
        localRunAdmissionRef.current = {
          threadId: sendThreadId,
          runId: null,
          settledBeforeResponse: false,
        };
      }
      submitBusyRef.current = true;
      updateCurrentThread((prev) => [...prev, pendingRenderMessage]);
      updateSeededTarget((prev) => [...prev, pendingRenderMessage]);

      updateCurrentRunState(() => {
        setIsProcessing(true);
        if (!pendingGateRef.current) {
          setPendingGate(null);
        }
      });

      try {
        const response = await sendMessage({
          threadId: sendThreadId,
          content,
          attachments: wireAttachments,
        });
        if (response?.outcome !== "rejected_busy") {
          touchThreadInCache({
            threadId: response?.thread_id || sendThreadId,
            messageContent: renderContent,
            updatedAt: pendingRecord.timestamp,
          });
        }
        // Refresh the sidebar only while the cached entry is missing
        // or title-less. Once the first-message title has appeared,
        // repeated sends do not need to refetch the whole thread list.
        if (threadNeedsSidebarRefresh(sendThreadId)) {
          queryClient.invalidateQueries({ queryKey: ["threads"] });
        }
        let runSettledBeforeResponse = false;
        if (response?.run_id && shouldTrackLocalRun) {
          const localRunAdmission = localRunAdmissionRef.current;
          runSettledBeforeResponse = Boolean(
            localRunAdmission &&
              localRunAdmission.threadId === sendThreadId &&
              localRunAdmission.runId === response.run_id &&
              localRunAdmission.settledBeforeResponse,
          );
          if (runSettledBeforeResponse) {
            localRunAdmissionRef.current = null;
          } else {
            localRunAdmissionRef.current = {
              threadId: sendThreadId,
              runId: response.run_id,
              settledBeforeResponse: false,
            };
          }
        } else if (shouldTrackLocalRun) {
          localRunAdmissionRef.current = null;
        }
        if (
          response?.run_id &&
          shouldRenderInCurrentThread &&
          !runSettledBeforeResponse
        ) {
          setActiveRun({
            runId: response.run_id,
            threadId: response.thread_id || sendThreadId,
            status: response.status || null,
            source: "local",
          });
        }
        const timelineMessageId =
          recordAcceptedMessageRef(
            pendingMessagesRef.current,
            pendingKey,
            optimisticId,
            response?.accepted_message_ref,
          ) || timelineMessageIdFromAcceptedRef(response?.accepted_message_ref);
        if (timelineMessageId) {
          const markAccepted = (prev) =>
            prev.map((m) =>
              m.id === optimisticId ? { ...m, timelineMessageId } : m,
            );
          updateCurrentThread(markAccepted);
          updateSeededTarget(markAccepted);
        }
        // When the thread was busy, the message is rejected (not deferred).
        // Mark the optimistic user message as failed and display the
        // server's notice (if present) as a system message so the user
        // knows to resend.
        if (response?.outcome === "rejected_busy") {
          if (shouldTrackLocalRun) {
            localRunAdmissionRef.current = null;
          }
          const markRejected = (prev) =>
            prev.map((m) =>
              m.id === optimisticId
                ? { ...m, isOptimistic: false, status: "error" }
                : m,
            );
          updateCurrentThread(markRejected);
          updateSeededTarget(markRejected);
          if (response?.notice) {
            const appendSystemNotice = (renderCurrent = shouldRenderInCurrentThread) => {
              const noticeMessage = {
                id: `system-rejected-${pendingSeqRef.current++}`,
                role: "system",
                content: response.notice,
                timestamp: new Date().toISOString(),
                isOptimistic: false,
              };
              const appendNotice = (prev) => [
                ...prev,
                noticeMessage,
              ];
              if (renderCurrent) setMessages(appendNotice);
              if (!renderCurrent || sendThreadId !== threadId) {
                seedThreadMessages(sendThreadId, appendNotice);
              }
            };
            const liveShouldRenderInCurrentThread =
              !threadIdRef.current || threadIdRef.current === sendThreadId;
            if (liveShouldRenderInCurrentThread) {
              const currentNoticeKey = busyNoticeKey(sendThreadId, pendingGateRef.current);
              if (currentNoticeKey) {
                setBusyGateNotice({
                  gateKey: currentNoticeKey,
                  content: response.notice,
                });
              } else {
                appendSystemNotice();
              }
            } else {
              appendSystemNotice(false);
            }
          }
          updateCurrentRunState(() => setIsProcessing(false));
          submitBusyRef.current = false;
        } else if (!response?.run_id) {
          if (shouldTrackLocalRun) {
            localRunAdmissionRef.current = null;
          }
          submitBusyRef.current = false;
        }
        return response;
      } catch (err) {
        if (shouldTrackLocalRun) {
          localRunAdmissionRef.current = null;
        }
        if (err.status === 429) {
          setCooldownUntil(Date.now() + retryAfterMs(err));
        }
        const markFailed = (prev) =>
          prev.map((m) =>
            m.id === optimisticId
              ? {
                  ...m,
                  isOptimistic: false,
                  status: "error",
                  error: err.message,
                }
              : m,
          );
        updateCurrentThread(markFailed);
        updateSeededTarget(markFailed);
        updateCurrentRunState(() => setIsProcessing(false));
        submitBusyRef.current = false;
        throw err;
      } finally {
        // Release the re-entrancy guard once the send POST settles — that is
        // the window it exists to protect (one in-flight submit at a time).
        // It must NOT stay held until the run settles: clearing it only in
        // `onRunSettled` (delivered over the *current* thread's SSE) deadlocks
        // the moment the user navigates to a new chat while a run is in
        // flight — that thread's SSE is torn down, its settle event never
        // arrives, the guard stays `true`, and every later send is silently
        // dropped. Blocking a resubmit into a still-running thread is the job
        // of the per-destination run guards above, not this.
        submitBusyRef.current = false;
        // Drop the optimistic from the pending ref unconditionally:
        // on success the confirmed row arrives via /timeline, and on
        // failure we mark the optimistic with `status: "error"` in
        // React state above — neither outcome needs the entry to
        // linger in `pendingMessagesRef`. Pending ids are `pending-N`
        // while server ids are `msg-<uuid>`, so id-based dedup in
        // `messagesFromTimeline` cannot reconcile a stale pending
        // against the server row that supersedes it.
        removePending(pendingMessagesRef.current, pendingKey, optimisticId);
      }
    },
    [
      threadId,
      pendingGate,
      pendingOnboarding,
      setMessages,
      seedThreadMessages,
      setIsProcessing,
      setPendingGate,
      setActiveRun,
    ],
  );
  sendRef.current = send;

  const resumeOnboardingAfterChannelConnected = React.useCallback(
    async (onboarding, event = {}) => {
      clearOnboardingAfterChannelConnected(onboarding);
    },
    [clearOnboardingAfterChannelConnected],
  );

  React.useEffect(() => {
    return subscribeChannelConnected((event) => {
      // A channel connected — here, in another tab, or on the extensions page.
      // Refresh the per-user connection snapshot so every chat's panel gate
      // (and the extensions UI) sees "connected" instead of the stale
      // "needs setup" cache; without this a freshly connected account can
      // still surface a "Connect" panel for up to the query staleTime.
      queryClient.invalidateQueries?.({ queryKey: ["extensions"] });
      queryClient.invalidateQueries?.({ queryKey: ["connectable-channels"] });
      const onboarding = pendingOnboardingRef.current;
      if (!connectionEventMatchesOnboarding(event, onboarding)) return;
      resumeOnboardingAfterChannelConnected(onboarding, event).catch((error) => {
        console.error("channel connection resume failed:", error);
      });
    });
  }, [resumeOnboardingAfterChannelConnected]);

  // v2 resolveGate signature: `(resolution, { always?, credentialRef? })`.
  // run_id and gate_ref come from the live `pendingGate` (set by the
  // gate / auth_required event) so the UI doesn't have to plumb them
  // through every approve-action call site.
  const resolveGate = React.useCallback(
    async (resolution, opts = {}) => {
      if (!pendingGate) return;
      const { runId, gateRef } = pendingGate;
      if (!runId || !gateRef) {
        throw new Error("resolveGate requires a pending gate with run_id and gate_ref");
      }
      const response = await resolveGateRequest({
        threadId,
        runId,
        gateRef,
        resolution,
        always: opts.always,
        credentialRef: opts.credentialRef,
      });
      const outcome = resolveGateOutcome(response);
      locallyResolvedGatesRef.current.set(`${runId}\n${gateRef}`, {
        resolution,
        outcome,
      });
      if (isDeclinedGateResolution(resolution) && outcome === "resumed") {
        failGateToolActivity(setMessages, pendingGate, toolActivityStateRef);
      }
      setPendingGate(null);
      if (outcome === "resumed") {
        setIsProcessing(true);
        setActiveRun({
          runId: response?.run_id || runId,
          threadId: response?.thread_id || threadId,
          status: response?.status || "queued",
        });
        return;
      }
      setIsProcessing(false);
      setActiveRun(null);
    },
    [pendingGate, threadId, setMessages, setActiveRun],
  );

  const submitAuthToken = React.useCallback(
    async (token) => {
      if (!pendingGate) {
        throw new Error("auth gate is no longer pending");
      }
      const { runId, gateRef, provider } = pendingGate;
      if (!runId || !gateRef || !provider) {
        throw new Error("auth gate is missing required credential metadata");
      }
      // `account_label` is optional on the prompt (gates.js defaults it to
      // an empty string), so don't gate submission on it — derive a sensible
      // label when the prompt didn't carry one.
      const accountLabel = pendingGate.accountLabel || `${provider} credential`;
      const gateKey = `${runId}\n${gateRef}`;
      if (authTokenSubmitRef.current.gateKey !== gateKey) {
        authTokenSubmitRef.current = {
          gateKey,
          credentialRef: null,
          inFlight: false,
        };
      }
      if (authTokenSubmitRef.current.inFlight) {
        throw new Error("auth token submission already in progress");
      }
      authTokenSubmitRef.current.inFlight = true;

      try {
        let credentialRef = authTokenSubmitRef.current.credentialRef;
        let submitted = null;
        if (!credentialRef) {
          submitted = await withAuthTokenTimeout((signal) =>
            submitManualToken({
              provider,
              accountLabel,
              token,
              threadId,
              runId,
              gateRef,
              signal,
            }),
          );
          credentialRef = submitted?.credential_ref;
          if (!credentialRef) {
            throw new Error("manual token submit returned no credential_ref");
          }
          authTokenSubmitRef.current.credentialRef = credentialRef;
        }

        if (!submitResponseResumedTurnGate(submitted)) {
          try {
            await withAuthTokenTimeout((signal) =>
              resolveGateRequest({
                threadId,
                runId,
                gateRef,
                resolution: "credential_provided",
                credentialRef,
                signal,
              }),
            );
          } catch (err) {
            throw credentialStoredGateResolutionError(err);
          }
        }

        authTokenSubmitRef.current = {
          gateKey: null,
          credentialRef: null,
          inFlight: false,
        };
        setPendingGate(null);
        setIsProcessing(true);
      } catch (err) {
        if (authTokenSubmitRef.current.gateKey === gateKey) {
          authTokenSubmitRef.current.inFlight = false;
        }
        throw err;
      }
    },
    [pendingGate, threadId],
  );

  const submitOnboardingPairing = React.useCallback(
    async (code) => {
      const onboarding = pendingOnboardingRef.current;
      if (!onboarding) {
        throw new Error("pairing is no longer pending");
      }
      const trimmed = String(code || "").trim();
      if (!trimmed) {
        throw new Error("pairing code is required");
      }
      const threadForResume = onboarding.threadId || threadId || null;
      const options = {
        threadId: threadForResume,
        requestId: onboarding.requestId || null,
      };
      const response = await redeemPairingCode(onboarding.extensionName, trimmed, options);
      if (response?.success === false) {
        throw new Error(response.message || "Pairing failed");
      }
      if (response?.resumeError) {
        // The connection succeeded (binding is durable), but the backend
        // couldn't continue this parked chat. The gate only clears when the
        // turn actually resumes, so it will stay pending — surface a distinct,
        // recoverable error instead of leaving the card spinning forever.
        const error = new Error("channel connection resume did not complete");
        error.resumeFailed = true;
        throw error;
      }
      const clearOnboarding = () => {
        if (onboarding.sourceMessageId) {
          dismissedOnboardingIdsRef.current.add(onboarding.sourceMessageId);
          persistDismissedOnboardingId(threadForResume, onboarding.sourceMessageId);
        }
        forgetChannelConnectionWaiter({
          channel: onboarding.extensionName,
          threadId: threadForResume,
          sourceMessageId: onboarding.sourceMessageId || null,
        });
        setPendingOnboarding(null);
      };
      if (threadForResume && !onboarding.requestId) {
        const continuation = await send(
          channelConnectionContinuationMessage(onboarding.extensionName),
          {
            threadId: threadForResume,
            bypassPendingOnboarding: true,
          },
        );
        // `send` returns null (or a rejected_busy outcome) without posting a
        // turn when thread admission blocks it — an in-flight submit or an
        // active run on the resume thread. The account is connected either way,
        // but the blocked request was NOT resumed: keep the waiter and panel
        // so the channel-connected event path (or a manual retry) can still
        // deliver the continuation instead of silently dropping it behind a
        // success response.
        if (continuation && continuation.outcome !== "rejected_busy") {
          clearOnboarding();
        }
        return response;
      }
      clearOnboarding();
      if (onboarding.requestId && threadForResume) {
        setIsProcessing(true);
      }
      return response;
    },
    [threadId, send, setPendingOnboarding, setIsProcessing],
  );

  const startOnboardingOAuth = React.useCallback(async () => {
    const onboarding = pendingOnboardingRef.current;
    if (!onboarding) {
      throw new Error("connection is no longer pending");
    }
    if (onboarding.strategy !== "oauth") {
      throw new Error("connection does not use OAuth");
    }
    const packageRef = { kind: "extension", id: onboarding.extensionName };
    const packageKey = onboarding.extensionName;
    const setup =
      typeof queryClient.fetchQuery === "function"
        ? await queryClient.fetchQuery({
            queryKey: ["extension-setup", packageKey],
            queryFn: () => fetchExtensionSetup(packageRef),
          })
        : await fetchExtensionSetup(packageRef);
    const secret = (setup?.secrets || []).find(
      (item) => (item?.setup?.kind || "manual_token") === "oauth",
    );
    if (!secret) {
      throw new Error("OAuth setup is unavailable for this channel");
    }
    const popup = window.open("about:blank", "_blank", "width=600,height=600");
    if (popup) popup.opener = null;
    try {
      const response = await startExtensionOauth(packageRef, secret);
      if (response?.success === false) {
        throw new Error(response.message || "OAuth setup failed");
      }
      if (!response?.authorization_url) {
        throw new Error("OAuth setup did not return an authorization URL");
      }
      if (!response?.flow_id) {
        throw new Error("OAuth setup did not return a flow id");
      }
      const opened = openAuthUrl(response.authorization_url, popup);
      if (!opened.ok) {
        throw new Error(
          opened.reason === "popup_blocked"
            ? "Authorization popup was blocked."
            : "Authorization URL must use HTTPS.",
        );
      }
      pendingOnboardingOauthFlowRef.current = {
        flowId: response.flow_id,
        channel: onboarding.extensionName,
        threadId: onboarding.threadId || threadId || null,
      };
      return response;
    } catch (error) {
      if (popup && !popup.closed) popup.close();
      throw error;
    }
  }, [threadId]);

  const dismissOnboardingPairing = React.useCallback(() => {
    const onboarding = pendingOnboardingRef.current;
    if (onboarding) {
      const threadForDismiss = onboarding.threadId || threadId;
      if (onboarding.sourceMessageId) {
        dismissedOnboardingIdsRef.current.add(onboarding.sourceMessageId);
        persistDismissedOnboardingId(threadForDismiss, onboarding.sourceMessageId);
      }
      forgetChannelConnectionWaiter({
        channel: onboarding.extensionName,
        threadId: threadForDismiss,
        sourceMessageId: onboarding.sourceMessageId || null,
      });
    }
    setPendingOnboarding(null);
  }, [threadId, setPendingOnboarding]);

  const cancelRun = React.useCallback(
    async (reason) => {
      const runId = activeRun?.runId;
      if (!runId || !threadId) return;
      setPendingGate(null);
      // Cancelling abandons any pairing panel for this thread: forget its waiter
      // and remember the dismissal so a later channel connect can't blast a
      // "Continue the previous request" into a chat the user explicitly cancelled,
      // and the durable activation card can't re-derive the panel.
      const onboarding = pendingOnboardingRef.current;
      if (onboarding) {
        const threadForCancel = onboarding.threadId || threadId;
        if (onboarding.sourceMessageId) {
          dismissedOnboardingIdsRef.current.add(onboarding.sourceMessageId);
          persistDismissedOnboardingId(threadForCancel, onboarding.sourceMessageId);
        }
        forgetChannelConnectionWaiter({
          channel: onboarding.extensionName,
          threadId: threadForCancel,
          sourceMessageId: onboarding.sourceMessageId || null,
        });
      }
      setPendingOnboarding(null);
      setIsProcessing(false);
      setActiveRun(null);
      submitBusyRef.current = false;
      const localRunAdmission = localRunAdmissionRef.current;
      if (
        localRunAdmission?.runId === runId ||
        localRunAdmission?.threadId === threadId
      ) {
        localRunAdmissionRef.current = null;
      }
      await cancelRunRequest({ threadId, runId, reason });
    },
    [activeRun, threadId],
  );

  const loadMore = React.useCallback(() => {
    if (hasMore && nextCursor) loadHistory(nextCursor);
  }, [hasMore, nextCursor, loadHistory]);

  // Fork-shape compatibility: `approve(requestId, action, kind)` from
  // chat.js. `requestId` and `kind` are v1 concepts the v2 stream
  // doesn't surface; the live `pendingGate` already carries
  // `runId` + `gateRef`, so the args are intentionally ignored and
  // the call is rerouted to v2 resolveGate.
  const approve = React.useCallback(
    async (_requestId, action, _kind) => {
      let resolution = "approved";
      let always = false;
      if (action === "deny") resolution = "denied";
      else if (action === "cancel") resolution = "cancelled";
      else if (action === "always") {
        resolution = "approved";
        always = true;
      }
      await resolveGate(resolution, { always });
    },
    [resolveGate],
  );

  const noop = React.useCallback(() => {}, []);
  const retryMessage = React.useCallback(
    async (message) => {
      if (!message || message.status !== "error") return;
      const content =
        typeof message.retryContent === "string"
          ? message.retryContent
          : typeof message.content === "string"
            ? message.content
            : "";
      const attachments = Array.isArray(message.retryAttachments)
        ? message.retryAttachments
        : [];
      if (!content && attachments.length === 0) return;

      const removeFailed = (prev) => prev.filter((item) => item.id !== message.id);
      const restoreFailedIfNoReplacement = (prev) => {
        const hasReplacement = prev.some(
          (item) =>
            item.id !== message.id &&
            item.role === "user" &&
            item.status === "error" &&
            item.retryContent === content,
        );
        return hasReplacement || prev.some((item) => item.id === message.id)
          ? prev
          : [...prev, message];
      };
      setMessages(removeFailed);
      if (threadId) seedThreadMessages(threadId, removeFailed);
      try {
        const response = await send(content, {
          threadId,
          attachments,
          displayContent:
            typeof message.retryDisplayContent === "string"
              ? message.retryDisplayContent
              : message.content,
        });
        if (response === null) {
          setMessages(restoreFailedIfNoReplacement);
          if (threadId) seedThreadMessages(threadId, restoreFailedIfNoReplacement);
        }
      } catch {
        // `send` renders a replacement failed optimistic message after
        // admission. If admission failed before that point, restore the
        // original retryable error bubble.
        setMessages(restoreFailedIfNoReplacement);
        if (threadId) seedThreadMessages(threadId, restoreFailedIfNoReplacement);
      }
    },
    [send, seedThreadMessages, setMessages, threadId],
  );

  return {
    // v2-native
    messages,
    isProcessing,
    pendingGate,
    pendingOnboarding: visiblePendingOnboarding,
    busyGateNotice,
    activeRun,
    sseStatus,
    historyLoading,
    historyLoadError,
    hasMore,
    cooldownSeconds,
    send,
    resolveGate,
    submitAuthToken,
    submitOnboardingPairing,
    startOnboardingOAuth,
    dismissOnboardingPairing,
    cancelRun,
    loadMore,
    // fork-shape compatibility — see comments above
    suggestions: [],
    setSuggestions: noop,
    retryMessage,
    approve,
    recoverHistory: noop,
    recoveryNotice: null,
  };
}

function isDeclinedGateResolution(resolution) {
  return resolution === "denied" || resolution === "cancelled";
}

function retryAfterMs(err) {
  const raw = err.headers?.get?.("Retry-After");
  const seconds = Number(raw);
  if (Number.isFinite(seconds) && seconds > 0) return seconds * 1000;
  return 2000;
}
