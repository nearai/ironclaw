import { useInfiniteQuery } from "@tanstack/react-query";
import { useApiClient } from "@/app";

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

// ConversationMessage and ConversationThread are exported via their interface declarations
