import { useQuery } from "@tanstack/react-query";
import React from "react";
import {
  createThread as createThreadRequest,
  deleteThread as deleteThreadRequest,
  listThreads,
} from "../../../lib/api";
import { queryClient } from "../../../lib/query-client";
import { normalizeSidebarTitle } from "../../../lib/thread-title";
import {
  appendThreadListPage,
  removeThreadList,
  upsertThreadInCache,
} from "../lib/thread-cache";

export function useThreads() {
  // No polling: the sidebar is kept current by local cache writes after
  // create/send and by invalidating after delete. The v2 deployment has no
  // out-of-band thread producers (no external channel, no background routine)
  // in this binary. The fork's 5s poll was inherited from a v1 multi-channel
  // context that doesn't apply here.
  const query = useQuery({
    queryKey: ["threads"],
    queryFn: ({ signal }) => listThreads({ signal }),
  });

  const [activeThreadId, setActiveThreadId] = React.useState(null);
  const [isCreating, setIsCreating] = React.useState(false);
  const [isLoadingMore, setIsLoadingMore] = React.useState(false);
  const [loadMoreError, setLoadMoreError] = React.useState(null);
  // In-flight create promises keyed by project scope. A single ref would
  // hand a create for project B the pending promise from project A and
  // mis-route the UI to the wrong project's thread; scope the dedup per
  // project so only true double-submits within one scope collapse.
  const createInFlightRef = React.useRef(new Map());
  const loadMoreInFlightRef = React.useRef(null);
  const loadMoreAbortRef = React.useRef(null);

  React.useEffect(() => () => {
    loadMoreAbortRef.current?.abort();
    loadMoreAbortRef.current = null;
  }, []);

  const handleCreateThread = React.useCallback(async (projectId) => {
    const scopeKey = projectId || "__global__";
    const inFlight = createInFlightRef.current.get(scopeKey);
    if (inFlight) return inFlight;

    setIsCreating(true);
    const createPromise = (async () => {
      try {
        const data = await createThreadRequest(projectId ? { projectId } : undefined);
        upsertThreadInCache(data?.thread);
        // RebornCreateThreadResponse → { thread: SessionThreadRecord }.
        // SessionThreadRecord uses `thread_id`, not `id`.
        const threadId = data?.thread?.thread_id;
        if (threadId) setActiveThreadId(threadId);
        return threadId;
      } finally {
        createInFlightRef.current.delete(scopeKey);
        setIsCreating(createInFlightRef.current.size > 0);
      }
    })();

    createInFlightRef.current.set(scopeKey, createPromise);
    return createPromise;
  }, []);

  const handleDeleteThread = React.useCallback(
    async (threadId) => {
      await deleteThreadRequest({ threadId });
      if (activeThreadId === threadId) {
        setActiveThreadId(null);
      }
      queryClient.setQueryData(["threads"], (data) => removeThreadList(data, threadId));
      queryClient.invalidateQueries({ queryKey: ["threads"] });
    },
    [activeThreadId]
  );

  const nextCursor = query.data?.next_cursor || null;
  const handleLoadMore = React.useCallback(() => {
    if (!nextCursor) return Promise.resolve();
    if (loadMoreInFlightRef.current) return loadMoreInFlightRef.current;

    setIsLoadingMore(true);
    setLoadMoreError(null);
    const requestedCursor = nextCursor;
    const controller = new AbortController();
    loadMoreAbortRef.current = controller;
    const request = listThreads({
      cursor: requestedCursor,
      signal: controller.signal,
    })
      .then((page) => {
        if (controller.signal.aborted) return;
        queryClient.setQueryData(["threads"], (data) =>
          appendThreadListPage(data, page, requestedCursor)
        );
        setLoadMoreError(null);
      })
      .catch((error) => {
        if (!controller.signal.aborted) setLoadMoreError(error);
      })
      .finally(() => {
        if (loadMoreInFlightRef.current === request) {
          loadMoreInFlightRef.current = null;
          loadMoreAbortRef.current = null;
          if (!controller.signal.aborted) setIsLoadingMore(false);
        }
      });
    loadMoreInFlightRef.current = request;
    return request;
  }, [nextCursor]);

  // Normalize v2 SessionThreadRecord → fork's expected shape:
  // - v2 carries `thread_id`; fork's thread-sidebar reads `thread.id`
  // - v2 has no `state`/`turn_count` fields (those are v1 metadata).
  //   Fill safe defaults so the UI's "Processing" pip and turn count
  //   never spuriously render.
  // - `created_at`/`updated_at` are emitted by the v2 backend now
  //   (updated_at bumped on every message append); they flow through
  //   the spread and drive the sidebar's activity ordering. The backend
  //   omits them (`skip_serializing_if`) for legacy records persisted
  //   before timestamps, so they arrive `undefined` and normalize to
  //   `null` here.
  const threads = React.useMemo(() => {
    const records = query.data?.threads || [];
    return records.map((record) => ({
      ...record,
      id: record.thread_id,
      title: normalizeSidebarTitle(record.title, record.thread_id),
      state: record.state || null,
      turn_count: record.turn_count || 0,
      updated_at: record.updated_at || null,
    }));
  }, [query.data]);

  return {
    threads,
    nextCursor,
    hasMore: Boolean(nextCursor),
    activeThreadId,
    setActiveThreadId,
    isLoading: query.isLoading,
    isCreating,
    isLoadingMore,
    loadMoreError,
    loadMore: handleLoadMore,
    createThread: handleCreateThread,
    deleteThread: handleDeleteThread,
  };
}
