import { useQuery } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import {
  createThread as createThreadRequest,
  listThreads,
} from "../../../lib/api.js";
import { queryClient } from "../../../lib/query-client.js";

// v2 has no delete-thread endpoint; the hook intentionally drops the
// delete affordance until the contract gains one.
export function useThreads() {
  // No polling: the sidebar refreshes via `queryClient.invalidateQueries`
  // after a local `createThread` succeeds, and the v2 deployment has no
  // out-of-band thread producers (no Telegram channel, no background
  // routine) in this binary. The fork's 5s poll was inherited from a v1
  // multi-channel context that doesn't apply here.
  const query = useQuery({
    queryKey: ["threads"],
    queryFn: () => listThreads({}),
  });

  const [activeThreadId, setActiveThreadId] = React.useState(null);
  const [isCreating, setIsCreating] = React.useState(false);
  const createInFlightRef = React.useRef(null);

  const handleCreateThread = React.useCallback(async () => {
    if (createInFlightRef.current) return createInFlightRef.current;

    setIsCreating(true);
    const createPromise = (async () => {
      try {
        const data = await createThreadRequest();
        queryClient.invalidateQueries({ queryKey: ["threads"] });
        // RebornCreateThreadResponse → { thread: SessionThreadRecord }.
        // SessionThreadRecord uses `thread_id`, not `id`.
        const threadId = data?.thread?.thread_id;
        if (threadId) setActiveThreadId(threadId);
        return threadId;
      } finally {
        setIsCreating(false);
        createInFlightRef.current = null;
      }
    })();

    createInFlightRef.current = createPromise;
    return createPromise;
  }, []);

  // TODO: v2 has no DELETE thread endpoint. The fork's sidebar
  // wires a delete affordance; we keep the function name on the
  // returned shape (so the sidebar component doesn't crash) but
  // make it a no-op. Issue #3886 Hard Non-Goal: no v1 calls.
  const deleteThread = React.useCallback(async (_threadId) => {
    return Promise.resolve({ success: false, message: "TODO: requires v2 delete-thread endpoint" });
  }, []);

  // Normalize v2 SessionThreadRecord → fork's expected shape:
  // - v2 carries `thread_id`; fork's thread-sidebar reads `thread.id`
  // - v2 has no `state`, `turn_count`, `updated_at` fields
  //   (those are v1 metadata). Fill safe defaults so the UI's
  //   "Processing" pip and turn count never spuriously render.
  const threads = React.useMemo(() => {
    const records = query.data?.threads || [];
    return records.map((record) => ({
      ...record,
      id: record.thread_id,
      state: record.state || null,
      turn_count: record.turn_count || 0,
      updated_at: record.updated_at || null,
    }));
  }, [query.data]);

  return {
    threads,
    nextCursor: query.data?.next_cursor || null,
    activeThreadId,
    setActiveThreadId,
    isLoading: query.isLoading,
    isCreating,
    isDeleting: false,
    createThread: handleCreateThread,
    deleteThread,
  };
}
