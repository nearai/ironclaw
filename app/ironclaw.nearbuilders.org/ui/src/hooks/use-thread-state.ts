import { useCallback, useEffect, useRef, useState } from "react";
import { useApiClient } from "@/app";
import { openIronclawEventSource } from "@/lib/ironclaw-sse";

export interface ThreadState {
  thread: {
    threadId: string;
    title?: string | null;
    scope?: {
      tenantId: string;
      agentId: string;
      projectId?: string;
    };
    createdByActorId?: string;
  };
  messages: Array<{
    messageId: string;
    kind: string;
    role?: string;
    content?: string;
    createdAt?: string;
    [key: string]: unknown;
  }>;
  summaryArtifacts?: Array<Record<string, unknown>>;
}

function extractMessages(data: Record<string, unknown>): ThreadState["messages"] {
  const state = data.state as Record<string, unknown> | undefined;
  if (!state) return [];
  const items = state.items as Array<Record<string, unknown>> | undefined;
  if (!items) return [];
  return items.map((item) => ({
    messageId: (item.message_id ?? item.messageId ?? item.id ?? "") as string,
    kind: (item.kind ?? "") as string,
    role: (item.role ?? undefined) as string | undefined,
    content: (item.content ?? undefined) as string | undefined,
    createdAt: (item.created_at ?? item.createdAt ?? undefined) as string | undefined,
    ...item,
  }));
}

export function useThreadState(
  threadId: string | null,
  threadMeta?: { title?: string | null; scope?: { tenantId: string; agentId: string; projectId?: string }; createdByActorId?: string },
) {
  const apiClient = useApiClient();
  const [state, setState] = useState<ThreadState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sseRef = useRef<{ close: () => void } | null>(null);

  const fetchTimeline = useCallback(async (id: string) => {
    try {
      const result = await apiClient.ironclaw.threads.getTimeline({ id, limit: 100 });
      return (result.data ?? []) as ThreadState["messages"];
    } catch {
      return [] as ThreadState["messages"];
    }
  }, [apiClient]);

  const connect = useCallback(async (id: string) => {
    sseRef.current?.close();
    setLoading(true);
    setError(null);

    const messages = await fetchTimeline(id);

    setState({
      thread: {
        threadId: id,
        title: threadMeta?.title,
        scope: threadMeta?.scope,
        createdByActorId: threadMeta?.createdByActorId,
      },
      messages,
      summaryArtifacts: [],
    });
    setLoading(false);

    const handle = openIronclawEventSource({
      threadId: id,
      onSnapshot: (data) => {
        const raw = data as Record<string, unknown>;
        const sseMessages = extractMessages(raw);
        if (sseMessages.length > 0) {
          setState((prev) => {
            if (!prev) {
              return { thread: { threadId: id }, messages: sseMessages };
            }
            const existingIds = new Set(prev.messages.map((m) => m.messageId));
            const newMessages = sseMessages.filter((m) => !existingIds.has(m.messageId));
            if (newMessages.length === 0) return prev;
            return { ...prev, messages: [...prev.messages, ...newMessages] };
          });
        }
      },
      onUpdate: (data) => {
        const raw = data as Record<string, unknown>;
        const sseMessages = extractMessages(raw);
        if (sseMessages.length > 0) {
          setState((prev) => {
            if (!prev) {
              return { thread: { threadId: id }, messages: sseMessages };
            }
            const existingIds = new Set(prev.messages.map((m) => m.messageId));
            const newMessages = sseMessages.filter((m) => !existingIds.has(m.messageId));
            if (newMessages.length === 0) return prev;
            return { ...prev, messages: [...prev.messages, ...newMessages] };
          });
        }
      },
      onEvent: () => {},
      onError: (status) => {
        if (status === "disconnected") {
          setError("Event stream disconnected");
        }
      },
      onOpen: () => setError(null),
    });

    sseRef.current = handle;
  }, [threadMeta, fetchTimeline]);

  useEffect(() => {
    if (threadId) {
      connect(threadId);
    } else {
      sseRef.current?.close();
      sseRef.current = null;
      setState(null);
      setLoading(false);
      setError(null);
    }
    return () => {
      sseRef.current?.close();
      sseRef.current = null;
    };
  }, [threadId, connect]);

  const rebuild = useCallback(async () => {
    if (threadId) {
      sseRef.current?.close();
      const messages = await fetchTimeline(threadId);
      setState((prev) => prev ? { ...prev, messages } : { thread: { threadId }, messages });
      const handle = openIronclawEventSource({
        threadId,
        onSnapshot: (data) => {
          const raw = data as Record<string, unknown>;
          const sseMessages = extractMessages(raw);
          if (sseMessages.length > 0) {
            setState((prev) => {
              if (!prev) return { thread: { threadId }, messages: sseMessages };
              const existingIds = new Set(prev.messages.map((m) => m.messageId));
              const newMessages = sseMessages.filter((m) => !existingIds.has(m.messageId));
              if (newMessages.length === 0) return prev;
              return { ...prev, messages: [...prev.messages, ...newMessages] };
            });
          }
        },
        onUpdate: (data) => {
          const raw = data as Record<string, unknown>;
          const sseMessages = extractMessages(raw);
          if (sseMessages.length > 0) {
            setState((prev) => {
              if (!prev) return { thread: { threadId }, messages: sseMessages };
              const existingIds = new Set(prev.messages.map((m) => m.messageId));
              const newMessages = sseMessages.filter((m) => !existingIds.has(m.messageId));
              if (newMessages.length === 0) return prev;
              return { ...prev, messages: [...prev.messages, ...newMessages] };
            });
          }
        },
        onEvent: () => {},
        onError: (status) => {
          if (status === "disconnected") setError("Event stream disconnected");
        },
        onOpen: () => setError(null),
      });
      sseRef.current = handle;
    }
  }, [threadId, fetchTimeline]);

  return { state, loading, error, rebuild };
}
