import { useMutation, useQuery } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import {
  fetchThreads,
  createThread,
  deleteThread as deleteThreadRequest,
} from "../../../lib/api.js";
import { queryClient } from "../../../lib/query-client.js";

export function useThreads() {
  const query = useQuery({
    queryKey: ["threads"],
    queryFn: fetchThreads,
    refetchInterval: 5000,
  });

  const [activeThreadId, setActiveThreadId] = React.useState(null);
  const [isCreating, setIsCreating] = React.useState(false);
  const createInFlightRef = React.useRef(null);

  const removeThreadFromCache = React.useCallback((threadId) => {
    queryClient.setQueryData(["threads"], (current) => {
      if (!current) return current;
      return {
        ...current,
        active_thread:
          current.active_thread === threadId ? null : current.active_thread,
        threads: (current.threads || []).filter(
          (thread) => thread.id !== threadId
        ),
      };
    });
  }, []);

  const handleCreateThread = React.useCallback(async () => {
    const activeFromServer = query.data?.active_thread || null;
    const candidateId = activeThreadId || activeFromServer;
    const candidate =
      candidateId && query.data?.threads ? query.data.threads.find((thread) => thread.id === candidateId) : null;

    if (candidateId && candidate && (candidate.turn_count || 0) === 0) {
      setActiveThreadId(candidateId);
      return candidateId;
    }

    if (createInFlightRef.current) {
      return createInFlightRef.current;
    }
    setIsCreating(true);
    const createPromise = (async () => {
      try {
        const data = await createThread();
        queryClient.invalidateQueries({ queryKey: ["threads"] });
        const threadId = data.thread_id || data.id;
        setActiveThreadId(threadId);
        return threadId;
      } finally {
        setIsCreating(false);
        createInFlightRef.current = null;
      }
    })();

    createInFlightRef.current = createPromise;
    return createPromise;
  }, [activeThreadId, query.data]);

  const deleteMutation = useMutation({
    mutationFn: ({ threadId }) => deleteThreadRequest(threadId),
    onMutate: ({ threadId }) => {
      removeThreadFromCache(threadId);
    },
    onSuccess: (_result, { threadId }) => {
      setActiveThreadId((current) => (current === threadId ? null : current));
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["threads"] });
    },
  });

  const handleDeleteThread = React.useCallback(
    async (threadId) => {
      await deleteMutation.mutateAsync({ threadId });
    },
    [deleteMutation]
  );

  return {
    threads: query.data?.threads || [],
    assistantThread: query.data?.assistant_thread,
    activeThreadId,
    setActiveThreadId,
    isLoading: query.isLoading,
    isCreating,
    isDeleting: deleteMutation.isPending,
    createThread: handleCreateThread,
    deleteThread: handleDeleteThread,
  };
}
