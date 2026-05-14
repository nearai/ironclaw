import { resolveGate, sendApproval, sendMessage } from "../../../lib/api.js";
import { React } from "../../../lib/html.js";
import { normalizeHistoryGate } from "../lib/gates.js";
import { useChatEvents } from "../lib/useChatEvents.js";
import { useHistory } from "./useHistory.js";
import { useSSE } from "./useSSE.js";

export function useChat(threadId) {
  const pendingMessagesRef = React.useRef(new Map());
  const pendingSeqRef = React.useRef(1);
  const [cooldownUntil, setCooldownUntil] = React.useState(0);
  const [now, setNow] = React.useState(Date.now());
  const [recoveryNotice, setRecoveryNotice] = React.useState(null);

  const getPendingMessages = React.useCallback(
    () => pendingMessagesRef.current.get(threadId || "__new__") || [],
    [threadId]
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
    [threadId]
  );

  const {
    messages,
    hasMore,
    oldestTimestamp,
    isLoading: historyLoading,
    inProgress,
    pendingGate: historyPendingGate,
    loadHistory,
    setMessages,
  } = useHistory(threadId, { getPendingMessages, setPendingMessages });

  const [isProcessing, setIsProcessing] = React.useState(false);
  const [pendingGate, setPendingGate] = React.useState(null);
  const [suggestions, setSuggestions] = React.useState([]);
  const cooldownSeconds = Math.max(0, Math.ceil((cooldownUntil - now) / 1000));

  React.useEffect(() => {
    if (!cooldownUntil) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [cooldownUntil]);

  React.useEffect(() => {
    setIsProcessing(Boolean(inProgress));
  }, [inProgress]);

  React.useEffect(() => {
    if (historyPendingGate) {
      setPendingGate(normalizeHistoryGate(historyPendingGate));
    }
  }, [historyPendingGate]);

  const recoverHistory = React.useCallback(async () => {
    setRecoveryNotice({ status: "loading", message: "Reloading thread history..." });
    await loadHistory();
    setRecoveryNotice({
      status: "done",
      message: "Recovered the latest thread history.",
    });
  }, [loadHistory]);

  const handleDoneWithoutResponse = React.useCallback(() => {
    setRecoveryNotice({
      status: "ready",
      message: "The turn finished before a response event arrived.",
    });
  }, []);

  const handleResponseComplete = React.useCallback(
    (responseThreadId) => {
      removeOldestPending(pendingMessagesRef.current, responseThreadId || threadId || "__new__");
    },
    [threadId]
  );

  const handleEvent = useChatEvents({
    threadId,
    setMessages,
    setIsProcessing,
    setPendingGate,
    setSuggestions,
    onDoneWithoutResponse: handleDoneWithoutResponse,
    onResponseComplete: handleResponseComplete,
  });

  const { status: sseStatus } = useSSE({
    onEvent: handleEvent,
    enabled: true,
  });

  const send = React.useCallback(
    async (
      content,
      { images = [], attachments = [], threadId: targetThreadId } = {}
    ) => {
      const sendThreadId = targetThreadId || threadId;
      const pendingKey = sendThreadId || "__new__";
      const displayContent =
        content ||
        (attachments.length > 0
          ? "(files attached)"
          : images.length > 0
            ? "(images attached)"
            : "(attachment)");
      const retryPayload = {
        content,
        images: images.map((img) => ({ ...img })),
        attachments: attachments.map((att) => ({ ...att })),
        threadId: sendThreadId,
      };
      const pendingRecord = {
        id: pendingSeqRef.current++,
        content: displayContent,
        copyText: buildPendingCopyText(displayContent, attachments),
        timestamp: Date.now(),
        images: images.map((img) => img.dataUrl),
        attachments: attachments.map(toAttachmentDisplay),
        retryPayload,
      };

      const optimisticId = `pending-${Date.now()}`;
      addPending(pendingMessagesRef.current, pendingKey, pendingRecord);
      setMessages((prev) => [
        ...prev,
        {
          id: optimisticId,
          role: "user",
          content: displayContent,
          timestamp: new Date().toISOString(),
          images: pendingRecord.images,
          attachments: pendingRecord.attachments,
          isOptimistic: true,
          retryPayload,
        },
      ]);

      setIsProcessing(true);
      setSuggestions([]);
      setPendingGate(null);

      try {
        const response = await sendMessage({
          content,
          threadId: sendThreadId,
          images: images.map((img) => ({
            media_type: img.media_type || img.mime_type,
            data: img.data || img.base64,
          })),
          attachments: attachments.map((att) => ({
            mime_type: att.mime_type,
            filename: att.filename,
            data_base64: att.data_base64 || att.base64,
          })),
        });
        if (response?.thread_id && response.thread_id !== pendingKey) {
          movePendingThread(pendingMessagesRef.current, pendingKey, response.thread_id);
        }
        return response;
      } catch (err) {
        removePending(pendingMessagesRef.current, pendingKey, pendingRecord.id);
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
                  retryPayload,
                }
              : m
          )
        );
        setIsProcessing(false);
        throw err;
      }
    },
    [threadId, setMessages]
  );

  const retryMessage = React.useCallback(
    async (message) => {
      const payload = message.retryPayload;
      if (!payload) return;
      setMessages((prev) => prev.filter((m) => m.id !== message.id));
      await send(payload.content, {
        images: payload.images || [],
        attachments: payload.attachments || [],
        threadId: payload.threadId || threadId,
      });
    },
    [send, setMessages, threadId]
  );

  const approve = React.useCallback(
    async (requestId, action, kind = "legacy") => {
      if (kind === "gate") {
        await resolveGate({ requestId, resolution: action, threadId });
      } else {
        await sendApproval({ requestId, action, threadId });
      }
      setPendingGate(null);
      setIsProcessing(true);
    },
    [threadId]
  );

  const resolveGateAction = React.useCallback(
    async (requestId, resolution) => {
      await resolveGate({ requestId, resolution, threadId });
      setPendingGate(null);
      setIsProcessing(true);
    },
    [threadId]
  );

  const loadMore = React.useCallback(() => {
    if (hasMore && oldestTimestamp) {
      loadHistory(oldestTimestamp);
    }
  }, [hasMore, oldestTimestamp, loadHistory]);

  return {
    messages,
    isProcessing,
    pendingGate,
    suggestions,
    sseStatus,
    historyLoading,
    hasMore,
    cooldownSeconds,
    recoveryNotice,
    send,
    retryMessage,
    approve,
    resolveGate: resolveGateAction,
    recoverHistory,
    loadMore,
    setSuggestions,
  };
}

function toAttachmentDisplay(att) {
  return {
    kind: att.kind || (att.mime_type?.startsWith("image/") ? "image" : "document"),
    filename: att.filename,
    mime_type: att.mime_type,
    size_label: att.size_label,
    preview_url: att.preview_url || null,
  };
}

function buildPendingCopyText(content, attachments) {
  const parts = content ? [content] : [];
  for (const attachment of attachments) {
    const suffix = [attachment.mime_type, attachment.size_label].filter(Boolean).join(" / ");
    parts.push(
      suffix
        ? `[Attachment] ${attachment.filename || "attachment"} (${suffix})`
        : `[Attachment] ${attachment.filename || "attachment"}`
    );
  }
  return parts.join("\n");
}

function addPending(store, key, record) {
  const existing = store.get(key) || [];
  store.set(key, [...existing, record]);
}

function removePending(store, key, pendingId) {
  const next = (store.get(key) || []).filter((record) => record.id !== pendingId);
  if (next.length > 0) store.set(key, next);
  else store.delete(key);
}

function removeOldestPending(store, key) {
  const existing = store.get(key) || [];
  existing.shift();
  if (existing.length > 0) store.set(key, existing);
  else store.delete(key);
}

function movePendingThread(store, fromKey, toKey) {
  const pending = store.get(fromKey);
  if (!pending || pending.length === 0) return;
  store.delete(fromKey);
  store.set(toKey, [...(store.get(toKey) || []), ...pending]);
}

function retryAfterMs(err) {
  const raw = err.headers?.get?.("Retry-After");
  const seconds = Number(raw);
  if (Number.isFinite(seconds) && seconds > 0) return seconds * 1000;
  return 2000;
}
