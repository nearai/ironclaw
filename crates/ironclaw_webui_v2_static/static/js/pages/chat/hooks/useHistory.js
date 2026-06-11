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
  });
  // Synchronous reentrancy guard — `isLoading` in state is async so
  // it can't gate overlapping calls (scroll-to-load + onRunCompleted
  // refetch can fire in the same tick). The ref flips before the
  // first await and clears in `finally` so a thrown timeline call
  // doesn't permanently wedge the next load.
  const loadingRef = React.useRef(false);
  // Tracks the currently-active thread so a fetch that resolves after
  // the user has switched threads doesn't clobber the live view (its
  // result still goes into the cache, keyed by its own thread id).
  const threadIdRef = React.useRef(threadId);
  threadIdRef.current = threadId;

  const loadHistory = React.useCallback(
    async (cursor) => {
      if (!threadId) {
        setState({ messages: [], nextCursor: null, isLoading: false });
        return;
      }
      if (loadingRef.current) return;
      loadingRef.current = true;
      setState((s) => ({ ...s, isLoading: true }));
      try {
        const data = await fetchTimeline({
          threadId,
          limit: PAGE_SIZE,
          cursor,
        });

        const pendingMessages = cursor ? [] : getPendingMessages?.() || [];
        const renderable = messagesFromTimeline(data.messages || [], pendingMessages);
        const nextCursor = data.next_cursor || null;

        // RebornTimelineResponse.next_cursor === null means we reached
        // the start of the thread.
        if (!cursor) setPendingMessages?.([]);

        // A full (non-paginated) load can be cached without the previous
        // state, so refresh the cache even if the user has since switched
        // threads.
        if (!cursor) {
          historyCache.set(cacheKey(threadId), { messages: renderable, nextCursor });
        }

        setState((prev) => {
          // Stale resolve for a thread that's no longer active: leave the
          // live view alone (the cache above already captured the result).
          if (threadIdRef.current !== threadId) return prev;
          const merged = cursor
            ? mergePage(renderable, prev.messages)
            : renderable;
          if (cursor) historyCache.set(cacheKey(threadId), { messages: merged, nextCursor });
          return {
            messages: merged,
            nextCursor,
            isLoading: false,
          };
        });
      } catch (err) {
        setState((s) =>
          threadIdRef.current === threadId ? { ...s, isLoading: false } : s,
        );
        // Stay loud — surface to the SPA error boundary rather than
        // silently masking timeline outages.
        console.error("Failed to load timeline:", err);
      } finally {
        loadingRef.current = false;
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
    });
    if (threadId) loadHistory();
  }, [threadId, loadHistory]);

  return {
    messages: state.messages,
    hasMore: Boolean(state.nextCursor),
    nextCursor: state.nextCursor,
    isLoading: state.isLoading,
    loadHistory,
    setMessages: (updater) =>
      setState((s) => {
        const messages =
          typeof updater === "function" ? updater(s.messages) : updater;
        // Keep the cache in step with optimistic sends and SSE-driven
        // updates so returning to the thread shows the latest messages.
        if (threadId) {
          historyCache.set(cacheKey(threadId), { messages, nextCursor: s.nextCursor });
        }
        return { ...s, messages };
      }),
  };
}

function mergePage(older, current) {
  const ids = new Set(current.map((m) => m.id));
  return [...older.filter((m) => !ids.has(m.id)), ...current];
}
