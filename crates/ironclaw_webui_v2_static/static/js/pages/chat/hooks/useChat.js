import {
  cancelRun as cancelRunRequest,
  createThread as createThreadRequest,
  resolveGate as resolveGateRequest,
  sendMessage,
  submitManualToken,
} from "../../../lib/api.js";
import {
  listConnectableChannels,
  looksLikeChannelConnectCommand,
  resolveChannelConnectCommand,
} from "../../../lib/channel-connect.js";
import { queryClient } from "../../../lib/query-client.js";
import { React } from "../../../lib/html.js";
import { useChatEvents } from "../lib/useChatEvents.js";
import {
  addPending,
  recordAcceptedMessageRef,
  removePending,
} from "../lib/pending-messages.js";
import { useHistory } from "./useHistory.js";
import { useSSE } from "./useSSE.js";

const AUTH_TOKEN_FLOW_TIMEOUT_MS = 30000;
const AUTH_GATE_CREDENTIAL_STORED_ERROR =
  "credential_stored_gate_resolution_failed";
const UNSUPPORTED_MULTIMODAL_PAYLOAD_ERROR =
  "webui_v2_multimodal_payload_unsupported";
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

function threadNeedsSidebarRefresh(threadId) {
  const cached = queryClient.getQueryData?.(["threads"]);
  const threads = cached?.threads;
  if (!Array.isArray(threads)) return true;
  const thread = threads.find((item) => item.thread_id === threadId || item.id === threadId);
  return !thread?.title;
}

function unsupportedMultimodalPayloadError() {
  const error = new Error("webchat v2 multimodal payload unsupported");
  error.safeErrorCode = UNSUPPORTED_MULTIMODAL_PAYLOAD_ERROR;
  return error;
}

function submitResponseResumedTurnGate(response) {
  return response?.continuation?.type === "turn_gate_resume";
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

async function resolveConnectAction(content) {
  if (!looksLikeChannelConnectCommand(content)) return null;
  try {
    const channelsResponse = await queryClient.fetchQuery({
      queryKey: ["connectable-channels"],
      queryFn: listConnectableChannels,
    });
    const channels = channelsResponse?.channels || [];
    return resolveChannelConnectCommand(content, channels);
  } catch (err) {
    console.error("Failed to resolve connectable channels:", err);
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
  const [channelConnectAction, setChannelConnectAction] = React.useState(null);

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
    setMessages,
  } = useHistory(threadId, { getPendingMessages, setPendingMessages });

  const [isProcessing, setIsProcessing] = React.useState(false);
  const [pendingGate, setPendingGate] = React.useState(null);
  const authTokenSubmitRef = React.useRef({
    gateKey: null,
    credentialRef: null,
    inFlight: false,
  });

  // Per-thread transient state must not leak across thread switches.
  // Without this reset, clicking "+ New" while the previous thread is
  // still processing renders the TypingIndicator on the empty new
  // thread. The SSE subscription for the new thread will set these
  // back to non-default values if that thread actually has an active
  // run / gate. `cooldownUntil` is intentionally not reset — it's a
  // rate-limit timer that applies across threads.
  React.useEffect(() => {
    setIsProcessing(false);
    setPendingGate(null);
    setActiveRun(null);
    setChannelConnectAction(null);
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
    // Reborn's projection bridge does not yet emit `Text` items for
    // assistant replies, so the SSE stream only delivers `run_status`.
    // On terminal success, refetch the timeline so the assistant
    // message that landed in the thread becomes visible in the UI.
    // Clear pending optimistic messages first so the real user
    // message from the server doesn't render alongside its
    // pre-submit optimistic twin.
    onRunCompleted: () => {
      setPendingMessages([]);
      loadHistory();
    },
  });

  const { status: sseStatus } = useSSE({
    threadId,
    onEvent: handleEvent,
    enabled: Boolean(threadId),
  });

  // Accepts the fork's call shape `{ images, attachments, threadId,
  // timezone }`. v2 SendMessage carries `content` only; image and
  // file payloads are rejected until the v2 contract grows typed
  // multimodal fields.
  //
  // v2 send-message requires `thread_id` as a path parameter — the
  // facade refuses to implicitly create a missing thread. When the
  // caller is on the landing screen (no active thread yet), we
  // eagerly POST `/threads` first and use the returned id. The
  // returned response carries `thread_id` so the chat.js navigation
  // hook can route to `/chat/<id>` after the first send.
  const send = React.useCallback(
    async (content, opts = {}) => {
      const { threadId: targetThreadId, images = [], attachments = [] } = opts;
      if (images.length > 0 || attachments.length > 0) {
        throw unsupportedMultimodalPayloadError();
      }

      const connectable = await resolveConnectAction(content);
      if (connectable) {
        setChannelConnectAction(connectable);
        return { channel_connect_action: connectable };
      }
      setChannelConnectAction(null);

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
        content,
        timestamp: new Date().toISOString(),
        isOptimistic: true,
      };
      addPending(pendingMessagesRef.current, pendingKey, pendingRecord);

      const optimisticId = pendingRecord.id;
      setMessages((prev) => [
        ...prev,
        {
          id: optimisticId,
          role: "user",
          content,
          timestamp: pendingRecord.timestamp,
          isOptimistic: true,
        },
      ]);

      setIsProcessing(true);
      setPendingGate(null);

      try {
        const response = await sendMessage({
          threadId: sendThreadId,
          content,
        });
        // Refresh the sidebar only while the cached entry is missing
        // or title-less. Once the first-message title has appeared,
        // repeated sends do not need to refetch the whole thread list.
        if (threadNeedsSidebarRefresh(sendThreadId)) {
          queryClient.invalidateQueries({ queryKey: ["threads"] });
        }
        if (response?.run_id) {
          setActiveRun({
            runId: response.run_id,
            threadId: response.thread_id || sendThreadId,
            status: response.status || null,
            source: "local",
          });
        }
        const timelineMessageId = recordAcceptedMessageRef(
          pendingMessagesRef.current,
          pendingKey,
          optimisticId,
          response?.accepted_message_ref,
        );
        if (timelineMessageId) {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === optimisticId ? { ...m, timelineMessageId } : m,
            ),
          );
        }
        return response;
      } catch (err) {
        if (err.status === 429) {
          setCooldownUntil(Date.now() + retryAfterMs(err));
        }
        setMessages((prev) =>
          prev.map((m) =>
            m.id === optimisticId
              ? {
                  ...m,
                  isOptimistic: false,
                  status: "error",
                  error: err.message,
                }
              : m,
          ),
        );
        setIsProcessing(false);
        throw err;
      } finally {
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
    [threadId, setMessages],
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
      await resolveGateRequest({
        threadId,
        runId,
        gateRef,
        resolution,
        always: opts.always,
        credentialRef: opts.credentialRef,
      });
      const shouldContinueProcessing =
        resolution === "approved" || resolution === "credential_provided";
      setPendingGate(null);
      setIsProcessing(shouldContinueProcessing);
      if (!shouldContinueProcessing) {
        setActiveRun(null);
      }
    },
    [pendingGate, threadId],
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

  // Fork chat.js expects these as stubs: v2 stream is deterministic
  // enough that retry / suggestions / recovery are not necessary in
  // local-dev. Wire them as no-ops so the chat UI renders without
  // additional branches.
  const noop = React.useCallback(() => {}, []);

  return {
    // v2-native
    messages,
    isProcessing,
    pendingGate,
    channelConnectAction,
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
    dismissChannelConnectAction: () => setChannelConnectAction(null),
    // fork-shape compatibility — see comments above
    suggestions: [],
    setSuggestions: noop,
    retryMessage: noop,
    approve,
    recoverHistory: noop,
    recoveryNotice: null,
  };
}

function retryAfterMs(err) {
  const raw = err.headers?.get?.("Retry-After");
  const seconds = Number(raw);
  if (Number.isFinite(seconds) && seconds > 0) return seconds * 1000;
  return 2000;
}
