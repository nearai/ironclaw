import { React } from "../../../lib/html.js";
import { fetchTimeline } from "../../../lib/api.js";
import { authScope } from "../../../lib/auth-scope.js";
import { messagesFromTimeline } from "../lib/history-messages.js";

const PAGE_SIZE = 50;

/* Session-lived per-thread message cache (survives component unmount).
 *
 * Returning to a conversation — e.g. after visiting Settings, which
 * unmounts the whole chat page — used to reset messages to [] and
 * re-fetch from scratch, flashing an empty list before the timeline
 * landed. This cache lets us render the last-known messages instantly
 * and refresh in the background (stale-while-revalidate), so the
 * content area no longer flickers. It is an in-memory cache, not a
 * source of truth; the /timeline endpoint remains authoritative. */
const historyCache = new Map();

// Cap the cache so a long SPA session visiting many threads can't grow it
// without bound. Map preserves insertion order, so re-inserting on write and
// evicting from the front gives simple LRU-ish behavior.
const MAX_CACHED_THREADS = 30;
function putCache(key, value) {
  historyCache.delete(key);
  historyCache.set(key, value);
  while (historyCache.size > MAX_CACHED_THREADS) {
    const oldest = historyCache.keys().next().value;
    historyCache.delete(oldest);
  }
}

// Namespace cache entries by the authenticated user so a session change in
// the same tab (sign-out/in, token swap, 401 re-auth) can't surface the
// previous user's cached conversations — a different identity reads under a
// different key and misses them.
function cacheKey(threadId) {
  return `${authScope()}:${threadId}`;
}

/// Drop all cached thread messages. Called on sign-out so a different user
/// logging in on the same tab (no full reload) can never observe the previous
/// session's cached conversations.
export function clearHistoryCache() {
  historyCache.clear();
}

export function useHistory(threadId, options = {}) {
  const { getPendingMessages, setPendingMessages } = options;
  const cached = threadId ? historyCache.get(cacheKey(threadId)) : null;
  const [state, setState] = React.useState({
    messages: cached?.messages || [],
    nextCursor: cached?.nextCursor || null,
    isLoading: false,
    // Non-null when an initial or cursor-load failed. Reset to null on a
    // successful load or when the threadId changes. The chat page renders
    // this as a user-visible error banner so timeline failures are never
    // silently swallowed.
    loadError: null,
  });
  // Synchronous reentrancy guard, tracked PER THREAD — `isLoading` in state is
  // async so it can't gate overlapping calls (scroll-to-load + onRunCompleted
  // refetch can fire in the same tick). It must be per-thread, not a single
  // boolean: a boolean held by an in-flight load of thread A would block a
  // switch to an uncached thread B, leaving B stuck loading. Each entry is
  // added before the first await and removed in `finally`.
  const loadingRef = React.useRef(new Set());
  // Tracks the currently-active thread so a fetch that resolves after
  // the user has switched threads doesn't clobber the live view (its
  // result still goes into the cache, keyed by its own thread id).
  const threadIdRef = React.useRef(threadId);
  threadIdRef.current = threadId;

  const loadHistory = React.useCallback(
    async (cursor) => {
      if (!threadId) {
        setState({ messages: [], nextCursor: null, isLoading: false, loadError: null });
        return;
      }
      if (loadingRef.current.has(threadId)) return;
      loadingRef.current.add(threadId);
      // Capture the issuing identity + cache key BEFORE the await. If the
      // user signs out / in (or swaps tokens) while this request is in
      // flight, the response belongs to the previous user: we must neither
      // render it for the new user nor write it under the new user's key.
      const issuingScope = authScope();
      const key = cacheKey(threadId);
      setState((s) => ({ ...s, isLoading: true }));
      try {
        const data = await fetchTimeline({
          threadId,
          limit: PAGE_SIZE,
          cursor,
        });

        // Identity changed during the fetch — discard the response entirely.
        if (authScope() !== issuingScope) return;

        const pendingMessages = cursor ? [] : getPendingMessages?.() || [];
        const renderable = messagesFromTimeline(data.messages || [], pendingMessages);
        const nextCursor = data.next_cursor || null;

        // RebornTimelineResponse.next_cursor === null means we reached
        // the start of the thread.
        if (!cursor) setPendingMessages?.([]);

        // A full (non-paginated) load can be cached without the previous
        // state, so refresh the cache even if the user has since switched
        // threads. Always under the issuing identity's key.
        if (!cursor) {
          putCache(key, { messages: renderable, nextCursor });
        }

        setState((prev) => {
          // Stale resolve for a thread that's no longer active: leave the
          // live view alone (the cache above already captured the result).
          if (threadIdRef.current !== threadId) return prev;
          const merged = cursor
            ? mergePage(renderable, prev.messages)
            : renderable;
          if (cursor) putCache(key, { messages: merged, nextCursor });
          return {
            messages: merged,
            nextCursor,
            isLoading: false,
            loadError: null,
          };
        });
      } catch (err) {
        console.error("Failed to load timeline:", err);
        // Identity changed mid-flight — the error isn't the new user's.
        if (authScope() !== issuingScope) return;
        // Stay loud — surface a user-visible error rather than silently
        // masking timeline outages. Ignore a stale resolve for a thread the
        // user already navigated away from (its data is already cached).
        setState((s) =>
          threadIdRef.current === threadId
            ? {
                ...s,
                isLoading: false,
                loadError: "Failed to load conversation history.",
              }
            : s,
        );
      } finally {
        loadingRef.current.delete(threadId);
      }
    },
    [threadId, getPendingMessages, setPendingMessages],
  );

  React.useEffect(() => {
    const entry = threadId ? historyCache.get(cacheKey(threadId)) : null;
    setState({
      messages: entry?.messages || [],
      nextCursor: entry?.nextCursor || null,
      // Only show the loading state when nothing is cached to show;
      // otherwise render the cached thread immediately and refresh in
      // the background so the content area doesn't flash empty.
      isLoading: Boolean(threadId) && !entry,
      loadError: null,
    });
    if (threadId) loadHistory();
  }, [threadId, loadHistory]);

  return {
    messages: state.messages,
    hasMore: Boolean(state.nextCursor),
    nextCursor: state.nextCursor,
    isLoading: state.isLoading,
    loadError: state.loadError,
    loadHistory,
    setMessages: (updater) =>
      setState((s) => {
        const messages =
          typeof updater === "function" ? updater(s.messages) : updater;
        // Keep the cache in step with optimistic sends and SSE-driven
        // updates so returning to the thread shows the latest messages.
        if (threadId) {
          putCache(cacheKey(threadId), { messages, nextCursor: s.nextCursor });
        }
        return { ...s, messages };
      }),
  };
}

function mergePage(older, current) {
  const ids = new Set(current.map((m) => m.id));
  return [...older.filter((m) => !ids.has(m.id)), ...current];
}
