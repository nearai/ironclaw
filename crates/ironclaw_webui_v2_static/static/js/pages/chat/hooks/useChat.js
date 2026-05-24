import {
  cancelRun as cancelRunRequest,
  createThread as createThreadRequest,
  resolveGate as resolveGateRequest,
  sendMessage,
} from "../../../lib/api.js";
import { queryClient } from "../../../lib/query-client.js";
import { React } from "../../../lib/html.js";
import { useChatEvents } from "../lib/useChatEvents.js";
import { useHistory } from "./useHistory.js";
import { useSSE } from "./useSSE.js";

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
  const [activeRun, setActiveRun] = React.useState(null);

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
    loadHistory,
    setMessages,
  } = useHistory(threadId, { getPendingMessages, setPendingMessages });

  const [isProcessing, setIsProcessing] = React.useState(false);
  const [pendingGate, setPendingGate] = React.useState(null);

  const cooldownSeconds = Math.max(0, Math.ceil((cooldownUntil - now) / 1000));

  React.useEffect(() => {
    if (!cooldownUntil) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [cooldownUntil]);

  const handleEvent = useChatEvents({
    threadId,
    setMessages,
    setIsProcessing,
    setPendingGate,
    setActiveRun,
  });

  const { status: sseStatus } = useSSE({
    threadId,
    onEvent: handleEvent,
    enabled: Boolean(threadId),
  });

  // Accepts the fork's call shape `{ images, attachments, threadId,
  // timezone }`. v2 SendMessage carries `content` only — images /
  // attachments / timezone are silently dropped until the v2
  // contract grows the matching fields. Composer UI still shows
  // attachment chips; this is the TODO surface.
  //
  // v2 send-message requires `thread_id` as a path parameter — the
  // facade refuses to implicitly create a missing thread. When the
  // caller is on the landing screen (no active thread yet), we
  // eagerly POST `/threads` first and use the returned id. The
  // returned response carries `thread_id` so the chat.js navigation
  // hook can route to `/chat/<id>` after the first send.
  const send = React.useCallback(
    async (content, opts = {}) => {
      const { threadId: targetThreadId } = opts;
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
        content,
        timestamp: new Date().toISOString(),
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
        if (response?.run_id) {
          setActiveRun({
            runId: response.run_id,
            threadId: response.thread_id || sendThreadId,
            status: response.status || null,
          });
        }
        return response;
      } catch (err) {
        removePending(pendingMessagesRef.current, pendingKey, optimisticId);
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
      setPendingGate(null);
      setIsProcessing(true);
    },
    [pendingGate, threadId],
  );

  const cancelRun = React.useCallback(
    async (reason) => {
      const runId = activeRun?.runId;
      if (!runId || !threadId) return;
      try {
        await cancelRunRequest({ threadId, runId, reason });
      } finally {
        setIsProcessing(false);
      }
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
    activeRun,
    sseStatus,
    historyLoading,
    hasMore,
    cooldownSeconds,
    send,
    resolveGate,
    cancelRun,
    loadMore,
    // fork-shape compatibility — see comments above
    suggestions: [],
    setSuggestions: noop,
    retryMessage: noop,
    approve,
    recoverHistory: noop,
    recoveryNotice: null,
  };
}

function addPending(store, key, record) {
  const existing = store.get(key) || [];
  store.set(key, [...existing, record]);
}

function removePending(store, key, pendingId) {
  const next = (store.get(key) || []).filter((r) => r.id !== pendingId);
  if (next.length > 0) store.set(key, next);
  else store.delete(key);
}

function retryAfterMs(err) {
  const raw = err.headers?.get?.("Retry-After");
  const seconds = Number(raw);
  if (Number.isFinite(seconds) && seconds > 0) return seconds * 1000;
  return 2000;
}
