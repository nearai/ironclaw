// @ts-nocheck
import { useQuery } from "@tanstack/react-query";
import { fetchThreadDetail } from "../lib/projects-api";

export function useProjectInspector({ threadId }) {
  const threadQuery = useQuery({
    queryKey: ["project-thread-detail", threadId],
    queryFn: () => fetchThreadDetail(threadId),
    enabled: Boolean(threadId),
    refetchInterval: threadId ? 4000 : false,
  });

  return {
    thread: threadQuery.data?.thread || null,
    inspectorType: threadId ? "thread" : null,
    isLoading: threadQuery.isLoading,
    isRefreshing: threadQuery.isFetching,
    error: threadQuery.error || null,
  };
}
