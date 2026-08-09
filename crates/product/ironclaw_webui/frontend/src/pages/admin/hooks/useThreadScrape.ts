// @ts-nocheck
import React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { saveBlob } from "../../../lib/download";
import {
  fetchThreadScrapeArtifact,
  fetchThreadScrapeRunArtifact,
  fetchThreadScrapeThreads,
} from "../lib/admin-api";

const THREAD_SCRAPE_PAGE_SIZE = 100;

function artifactFilename(prefix, id) {
  const safeId = String(id || "artifact").replace(/[^a-zA-Z0-9._-]/g, "_");
  return `ironclaw-${prefix}-${safeId}.json`;
}

function saveArtifact(artifact, filename) {
  saveBlob(
    // Compact JSON, not pretty-printed: the largest thread artifacts can
    // reach ~16MiB, and the 2-space indentation doubles the transient copy.
    new Blob([`${JSON.stringify(artifact)}\n`], {
      type: "application/json",
    }),
    filename,
  );
}

/**
 * Data layer for the admin thread-scraping panel, mirroring the sibling
 * `useAdminUsers` hook: the initial thread page and the selected artifact are
 * react-query queries keyed by target user (and thread), so query keying —
 * not panel-local bookkeeping — guarantees that a late response for one
 * user/thread can never render under another selection. Load-more pages and
 * downloads stay plain imperative calls.
 *
 * Authorization and same-tenant target scope are revalidated server-side on
 * every request; the panel never sees another tenant's data.
 */
export function useThreadScrape(userId) {
  const queryClient = useQueryClient();
  const [threads, setThreads] = React.useState([]);
  const [nextCursor, setNextCursor] = React.useState(null);
  const [selectedThreadId, setSelectedThreadId] = React.useState("");
  const [artifact, setArtifact] = React.useState(null);
  const [isLoadingMore, setIsLoadingMore] = React.useState(false);
  const [downloadingRunId, setDownloadingRunId] = React.useState("");
  const [errorKey, setErrorKey] = React.useState("");
  const loadMoreAbortRef = React.useRef(null);
  const runAbortRef = React.useRef(null);

  const listQuery = useQuery({
    queryKey: ["admin", "threadScrape", "threads", userId],
    queryFn: ({ signal }) =>
      fetchThreadScrapeThreads(userId, {
        limit: THREAD_SCRAPE_PAGE_SIZE,
        signal,
      }),
    retry: false,
  });

  const artifactQuery = useQuery({
    queryKey: ["admin", "threadScrape", "artifact", userId, selectedThreadId],
    queryFn: ({ signal }) =>
      fetchThreadScrapeArtifact(userId, selectedThreadId, { signal }),
    enabled: Boolean(selectedThreadId),
    retry: false,
  });

  // Reset every per-target selection when the target user changes so one
  // user's transcript can never render under another user's detail panel.
  React.useEffect(() => {
    setThreads([]);
    setNextCursor(null);
    setSelectedThreadId("");
    setArtifact(null);
    setDownloadingRunId("");
    setErrorKey("");
    setIsLoadingMore(false);
    loadMoreAbortRef.current?.abort();
    loadMoreAbortRef.current = null;
    runAbortRef.current?.abort();
    runAbortRef.current = null;
  }, [userId]);

  // Cancel every in-flight request for the previous target user when the
  // target changes: react-query keeps inactive queries cached, so without an
  // explicit cancel the old queryFn's signal would stay live and the request
  // would finish server-side.
  const previousUserIdRef = React.useRef(userId);
  React.useEffect(() => {
    if (previousUserIdRef.current === userId) return;
    const previous = previousUserIdRef.current;
    previousUserIdRef.current = userId;
    queryClient.cancelQueries({
      queryKey: ["admin", "threadScrape", "threads", previous],
    });
    queryClient.cancelQueries({
      queryKey: ["admin", "threadScrape", "artifact", previous],
    });
  }, [queryClient, userId]);

  React.useEffect(() => {
    if (listQuery.isSuccess) {
      setThreads(Array.isArray(listQuery.data?.threads) ? listQuery.data.threads : []);
      setNextCursor(listQuery.data?.next_cursor ?? null);
    }
  }, [listQuery.isSuccess, listQuery.data]);

  React.useEffect(() => {
    if (artifactQuery.isSuccess) {
      setArtifact(artifactQuery.data);
    }
  }, [artifactQuery.isSuccess, artifactQuery.data]);

  // Cancelled requests must not surface errors: a target switch aborts the
  // previous user's requests, and their late rejections are inert.
  React.useEffect(() => {
    if (listQuery.error && !listQuery.isCancelled) {
      setErrorKey("admin.threadScraping.loadFailed");
    }
  }, [listQuery.error, listQuery.isCancelled]);

  React.useEffect(() => {
    if (artifactQuery.error && !artifactQuery.isCancelled) {
      setErrorKey("admin.threadScraping.loadFailed");
    }
  }, [artifactQuery.error, artifactQuery.isCancelled]);

  const selectThread = (threadId) => {
    runAbortRef.current?.abort();
    runAbortRef.current = null;
    setErrorKey("");
    if (threadId === selectedThreadId) {
      // Re-selecting the same thread re-runs the artifact query (e.g. after
      // a transient failure), matching the imperative fetch the panel
      // previously fired on every click.
      queryClient.refetchQueries({
        queryKey: ["admin", "threadScrape", "artifact", userId, threadId],
      });
      return;
    }
    setSelectedThreadId(threadId);
    setArtifact(null);
  };

  const loadMore = async () => {
    if (!nextCursor || isLoadingMore) return;
    const cursor = nextCursor;
    const controller = new AbortController();
    loadMoreAbortRef.current = controller;
    setIsLoadingMore(true);
    setErrorKey("");
    try {
      const response = await fetchThreadScrapeThreads(userId, {
        limit: THREAD_SCRAPE_PAGE_SIZE,
        cursor,
        signal: controller.signal,
      });
      if (controller.signal.aborted) return;
      const page = Array.isArray(response?.threads) ? response.threads : [];
      setThreads((current) => {
        const ids = new Set(current.map((thread) => thread.thread_id));
        return [...current, ...page.filter((thread) => !ids.has(thread.thread_id))];
      });
      setNextCursor(response?.next_cursor ?? null);
    } catch (requestError) {
      if (!controller.signal.aborted && requestError?.name !== "AbortError") {
        setErrorKey("admin.threadScraping.loadFailed");
      }
    } finally {
      if (loadMoreAbortRef.current === controller) {
        loadMoreAbortRef.current = null;
        setIsLoadingMore(false);
      }
    }
  };

  const downloadRun = async (runId) => {
    if (!selectedThreadId || !runId || downloadingRunId) return;
    const controller = new AbortController();
    runAbortRef.current = controller;
    setDownloadingRunId(runId);
    setErrorKey("");
    try {
      const runArtifact = await fetchThreadScrapeRunArtifact(
        userId,
        selectedThreadId,
        runId,
        { signal: controller.signal },
      );
      // The target user or selected thread changed while the request was in
      // flight: the request belongs to an earlier selection, so neither save
      // the artifact nor touch the new selection's state.
      if (runAbortRef.current !== controller) return;
      saveArtifact(runArtifact, artifactFilename("run", runId));
    } catch {
      if (runAbortRef.current === controller) {
        setErrorKey("admin.threadScraping.downloadFailed");
      }
    } finally {
      if (runAbortRef.current === controller) {
        runAbortRef.current = null;
        setDownloadingRunId("");
      }
    }
  };

  const downloadThreadArtifact = () => {
    if (!artifact) return;
    saveArtifact(artifact, artifactFilename("thread", artifact.thread_id));
  };

  return {
    threads,
    nextCursor,
    isLoading: listQuery.isLoading,
    isLoadingMore,
    isLoadingArtifact: artifactQuery.isLoading,
    selectedThreadId,
    selectThread,
    artifact,
    loadMore,
    downloadingRunId,
    downloadRun,
    downloadThreadArtifact,
    errorKey,
  };
}
