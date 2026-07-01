import {
  cancelRun as cancelRunRequest,
  createThread as createThreadRequest,
  resolveGate as resolveGateRequest,
  sendMessage,
  submitManualToken,
} from "../../../lib/api.js";
import { React } from "../../../lib/html.js";
import { useChatEvents } from "../lib/useChatEvents.js";
import { touchThreadInCache, upsertThreadInCache } from "../lib/thread-cache.js";
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

function busyNoticeKey(threadId, gate) {
  if (!threadId || !gate?.runId || !gate?.gateRef) return null;
  return `${threadId}\n${gate.runId}\n${gate.gateRef}`;
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
  const [busyGateNotice, setBusyGateNotice] = React.useState(null);
  const setPendingGate = React.useCallback((next) => {
    const current = pendingGateRef.current;
    const value =
      typeof next === "function" ? next(current) : next;
    if (Object.is(value, current)) return;
    pendingGateRef.current = value;
    setPendingGateState(value);
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
  }, [threadId]);

  const cooldownSeconds = Math.max(0, Math.ceil((cooldownUntil - now) / 1000));
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
    if (!isPendingOAuthGate(pendingGate)) return;
    const listeningSince = Date.now();

    const handleCompletion = (payload) => {
      if (!oauthCompletionMatchesGate(payload, pendingGate, listeningSince)) return;
      setPendingGate((current) => (isPendingOAuthGate(current) ? null : current));
      setIsProcessing(true);
    };

    let channel = null;
    if (typeof window.BroadcastChannel === "function") {
      channel = new window.BroadcastChannel(OAUTH_CALLBACK_CHANNEL);
      channel.onmessage = (event) => handleCompletion(event.data);
    }

    const onStorage = (event) => {
      if (event.key !== OAUTH_CALLBACK_STORAGE_KEY) return;
      handleCompletion(parseOAuthCallbackStoragePayload(event.newValue));
    };

    window.addEventListener("storage", onStorage);
    handleCompletion(
      parseOAuthCallbackStoragePayload(
        window.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY),
      ),
    );
    const timer = window.setInterval(() => {
      handleCompletion(
        parseOAuthCallbackStoragePayload(
          window.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY),
        ),
      );
    }, 500);
    return () => {
      window.clearInterval(timer);
      if (channel) channel.close();
      window.removeEventListener("storage", onStorage);
    };
  }, [pendingGate]);

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
        displayContent,
      } = opts;
      const wireAttachments = stagedAttachments.map(toWireAttachment);
      const renderAttachments = stagedAttachments.map(toRenderAttachment);
      const renderContent =
        typeof displayContent === "string" ? displayContent : content;

      if (pendingGate || pendingGateRef.current) {
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
        upsertThreadInCache(created?.thread);
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
        if (err && typeof err === "object") {
          err.optimisticMessageId = optimisticId;
          err.optimisticThreadId = sendThreadId;
        }
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
      setMessages,
      seedThreadMessages,
      setIsProcessing,
      setPendingGate,
      setActiveRun,
    ],
  );

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

  const cancelRun = React.useCallback(
    async (reason) => {
      const runId = activeRun?.runId;
      if (!runId || !threadId) return;
      setPendingGate(null);
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
      } catch (err) {
        if (err?.optimisticMessageId) {
          setMessages(removeFailed);
          if (threadId) seedThreadMessages(threadId, removeFailed);
          return;
        }
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
