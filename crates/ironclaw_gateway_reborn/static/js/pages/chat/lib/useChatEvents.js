import { React } from "../../../lib/html.js";
import { rememberGeneratedImage } from "./history-messages.js";

export function useChatEvents({
  threadId,
  setMessages,
  setIsProcessing,
  setPendingGate,
  setSuggestions,
  onDoneWithoutResponse,
  onResponseComplete,
}) {
  const streamingIdRef = React.useRef(null);
  const seenEventIdsRef = React.useRef([]);
  const turnResponseReceivedRef = React.useRef(false);

  return React.useCallback(
    (event) => {
      const { type } = event || {};
      let { data } = event || {};
      if (data == null) return;
      if (typeof data !== "object") {
        if (type === "status") {
          data = { message: data };
        } else {
          return;
        }
      }
      if (event.lastEventId && rememberSeenEvent(seenEventIdsRef, event.lastEventId)) {
        return;
      }
      if (data.thread_id && threadId && data.thread_id !== threadId) return;

      if (type === "stream_chunk") {
        turnResponseReceivedRef.current = true;
        setMessages((prev) => upsertStreamingMessage(prev, data, streamingIdRef));
        setIsProcessing(true);
        return;
      }

      if (type === "response") {
        turnResponseReceivedRef.current = true;
        setMessages((prev) => finishStreamingMessage(prev, data, streamingIdRef));
        setIsProcessing(false);
        onResponseComplete?.(data.thread_id);
        return;
      }

      if (type === "tool_started") {
        setMessages((prev) => upsertToolStartedMessage(prev, data));
        setIsProcessing(true);
        return;
      }

      if (type === "tool_completed" || type === "tool_result") {
        setMessages((prev) => updateToolMessage(prev, data, type));
        return;
      }

      if (type === "gate_required" || type === "approval_needed") {
        setPendingGate(toPendingGate(type, data));
        setIsProcessing(false);
        return;
      }

      if (type === "gate_resolved") {
        setPendingGate(null);
        return;
      }

      if (type === "error") {
        setMessages((prev) => [...prev, errorMessage(data)]);
        setIsProcessing(false);
        turnResponseReceivedRef.current = false;
        return;
      }

      if (type === "image_generated") {
        rememberGeneratedImage(data.thread_id || threadId, data.event_id, data.data_url, data.path);
        setMessages((prev) => [...prev, generatedImageMessage(data)]);
        return;
      }

      if (type === "suggestions") {
        setSuggestions(data.suggestions || []);
        return;
      }

      if (type === "status") {
        const message = data.message || data.content;
        if (message === "Done" && !turnResponseReceivedRef.current) {
          onDoneWithoutResponse?.();
        }
        if (["Done", "Idle", "Interrupted", "Rejected", "Tool call denied."].includes(message)) {
          setIsProcessing(false);
          turnResponseReceivedRef.current = false;
        }
      }
    },
    [
      threadId,
      setMessages,
      setIsProcessing,
      setPendingGate,
      setSuggestions,
      onDoneWithoutResponse,
      onResponseComplete,
    ]
  );
}

function rememberSeenEvent(seenEventIdsRef, eventId) {
  const seen = seenEventIdsRef.current;
  if (seen.includes(eventId)) return true;
  seen.push(eventId);
  if (seen.length > 400) seen.splice(0, seen.length - 400);
  return false;
}

function upsertStreamingMessage(messages, data, streamingIdRef) {
  if (streamingIdRef.current) {
    const index = messages.findIndex((message) => message.id === streamingIdRef.current);
    if (index >= 0) {
      const next = [...messages];
      next[index] = { ...next[index], content: next[index].content + data.content };
      return next;
    }
  }

  const id = `stream-${Date.now()}`;
  streamingIdRef.current = id;
  return [...messages, { id, role: "assistant", content: data.content, timestamp: new Date().toISOString(), isStreaming: true }];
}

function finishStreamingMessage(messages, data, streamingIdRef) {
  if (streamingIdRef.current) {
    const index = messages.findIndex((message) => message.id === streamingIdRef.current);
    if (index >= 0) {
      const next = [...messages];
      next[index] = { ...next[index], content: data.content, isStreaming: false };
      streamingIdRef.current = null;
      return next;
    }
  }

  return [...messages, { id: `resp-${Date.now()}`, role: "assistant", content: data.content, timestamp: new Date().toISOString(), isStreaming: false }];
}

function toolStartedMessage(data) {
  return {
    id: `tool-start-${data.call_id || Date.now()}`,
    role: "tool_activity",
    content: "",
    toolName: data.name,
    toolStatus: "running",
    toolDetail: data.detail,
    callId: data.call_id,
    timestamp: new Date().toISOString(),
  };
}

function upsertToolStartedMessage(messages, data) {
  if (!data.call_id) return [...messages, toolStartedMessage(data)];
  const exists = messages.some(
    (message) => message.role === "tool_activity" && message.callId === data.call_id
  );
  return exists ? messages : [...messages, toolStartedMessage(data)];
}

function updateToolMessage(messages, data, type) {
  const index = messages.findIndex((message) => message.role === "tool_activity" && message.callId === data.call_id);
  if (index < 0) return messages;

  const next = [...messages];
  next[index] =
    type === "tool_result"
      ? { ...next[index], toolResultPreview: data.preview }
      : {
          ...next[index],
          toolStatus: data.success ? "success" : "error",
          toolError: data.error,
          toolDurationMs: data.duration_ms,
          toolParameters: data.parameters,
        };
  return next;
}

function toPendingGate(type, data) {
  const gate = {
    requestId: data.request_id,
    toolName: data.tool_name,
    description: data.description,
    parameters: data.parameters,
    allowAlways: data.allow_always,
  };

  if (type === "gate_required") {
    return { ...gate, kind: "gate", gateName: data.gate_name, extensionName: data.extension_name };
  }
  return { ...gate, kind: "legacy", gateName: "approval" };
}

function errorMessage(data) {
  return { id: `err-${Date.now()}`, role: "error", content: data.message, timestamp: new Date().toISOString() };
}

function generatedImageMessage(data) {
  return {
    id: `img-${data.event_id}`,
    role: "image",
    content: "",
    generatedImages: [{ data_url: data.data_url, path: data.path }],
    timestamp: new Date().toISOString(),
  };
}
