import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { useApiClient } from "@/app";
import { openIronclawEventSource } from "@/lib/ironclaw-sse";

export interface ConversationMessage {
  id: string;
  threadId: string;
  role: "user" | "assistant";
  text: string;
  createdAt: string | null;
  status: "submitted" | "finalized" | "failed";
  sequence: number;
  runId: string | null;
}

export interface ConversationThread {
  threadId: string;
  title: string | null;
  tenantId: string;
  agentId: string;
  projectId: string | null;
  createdByActorId: string;
}

const MESSAGES_KEY = (threadId: string) => ["conversation", "messages", threadId] as const;
const THREADS_KEY = ["conversation", "threads"] as const;

export function useConversationThreads() {
  const apiClient = useApiClient();

  return useInfiniteQuery({
    queryKey: THREADS_KEY,
    queryFn: async () => {
      const data = await (apiClient as any).conversation.listThreads();
      return { threads: (data?.data ?? []) as ConversationThread[], nextCursor: null };
    },
    initialPageParam: null as string | null,
    getNextPageParam: () => null,
    staleTime: 30_000,
  });
}

export function useConversationMessages(threadId: string | null) {
  const apiClient = useApiClient();

  return useInfiniteQuery({
    queryKey: MESSAGES_KEY(threadId ?? ""),
    queryFn: async ({ pageParam }) => {
      if (!threadId) return { messages: [], nextCursor: null, hasMore: false, total: 0 };
      const page = await (apiClient as any).conversation.getMessages({
        threadId,
        cursor: pageParam ?? undefined,
        limit: 100,
      });
      return {
        messages: (page?.messages ?? []) as ConversationMessage[],
        nextCursor: (page?.nextCursor as string | null) ?? null,
        hasMore: (page?.hasMore as boolean) ?? false,
        total: (page?.total as number) ?? 0,
      };
    },
    initialPageParam: null as string | null,
    getNextPageParam: (lastPage) => lastPage.hasMore ? lastPage.nextCursor : null,
    enabled: !!threadId,
    staleTime: 10_000,
  });
}

export function useSendConversationMessage(threadId: string | null) {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({ content, clientActionId }: { content: string; clientActionId: string }) => {
      if (!threadId) throw new Error("No active thread");
      return (apiClient as any).conversation.sendMessage({ threadId, content, clientActionId });
    },
    onSuccess: () => {
      if (threadId) {
        queryClient.invalidateQueries({ queryKey: MESSAGES_KEY(threadId) });
      }
    },
  });
}

export interface PendingRun {
  id: string;
  runId: string;
  threadId: string;
  content: string;
  submittedAt: number;
}

export function useConversationStream(threadId: string | null) {
  const queryClient = useQueryClient();
  const sseRef = useRef<{ close: () => void } | null>(null);

  useEffect(() => {
    if (!threadId) return;

    sseRef.current?.close();

    const handle = openIronclawEventSource({
      threadId,
      onSnapshot: () => {
        queryClient.invalidateQueries({ queryKey: MESSAGES_KEY(threadId) });
      },
      onUpdate: () => {
        queryClient.invalidateQueries({ queryKey: MESSAGES_KEY(threadId) });
      },
      onEvent: () => {},
      onError: () => {},
    });

    sseRef.current = handle;

    return () => {
      handle.close();
      sseRef.current = null;
    };
  }, [threadId, queryClient]);

  return null;
}

// ConversationMessage and ConversationThread are exported via their interface declarations
