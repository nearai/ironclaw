// @ts-nocheck
import React from "react";
import { fetchTimeline } from "../../../lib/api";
import { authScope } from "../../../lib/auth-scope";
import { messagesFromTimeline } from "../lib/history-messages";
import {
  carryFinalAssistantOrderFlags,
  isFinalAssistantMessage,
  isRunActivityMessage,
} from "../lib/stream-order-memory";

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
    // The thread `messages` belong to. Set together with `messages` everywhere,
    // so a consumer can tell when `messages` still holds the previous thread's
    // timeline: it lags `threadId` by a render on a thread switch, until the
    // thread-change effect below swaps it in.
    messagesThreadId: threadId || null,
    nextCursor: cached?.nextCursor || null,
    sequenceWindow:
      cached?.sequenceWindow || timelineSequenceWindow(cached?.messages || []),
    isLoading: false,
    // Non-null when an initial or cursor-load failed. Reset to null on a
    // successful load or when the threadId changes. The chat page renders
    // this as a user-visible error banner so timeline failures are never
    // silently swallowed.
    loadError: null,
  });
  const [stateThreadId, setStateThreadId] = React.useState(threadId);
  if (stateThreadId !== threadId) {
    const entry = threadId ? historyCache.get(cacheKey(threadId)) : null;
    setStateThreadId(threadId);
    setState({
      messages: entry?.messages || [],
      messagesThreadId: threadId || null,
      nextCursor: entry?.nextCursor || null,
      sequenceWindow:
        entry?.sequenceWindow || timelineSequenceWindow(entry?.messages || []),
      isLoading: Boolean(threadId) && !entry,
      loadError: null,
    });
  }
  // Synchronous reentrancy guard, tracked PER THREAD AND PAGE — `isLoading` in
  // state is async so it can't gate overlapping calls (scroll-to-load +
  // onRunSettled refetch can fire in the same tick). It must be per-page, not
  // just per-thread: a background latest-page refresh must not cause an
  // explicit "load older" cursor request for the same thread to be dropped.
  // Each entry is added before the first await and removed in `finally`.
  const loadingRef = React.useRef(new Set());
  // Tracks the currently-active thread so a fetch that resolves after
  // the user has switched threads doesn't clobber the live view (its
  // result still goes into the cache, keyed by its own thread id).
  const threadIdRef = React.useRef(threadId);
  threadIdRef.current = threadId;

  const loadHistory = React.useCallback(
    async (cursor, loadOptions = {}) => {
      // `preserveClientOnly` keeps client-synthesized messages that never
      // appear in the timeline (run-failure `err-*` bubbles) when a full
      // reload replaces the list. A settle-triggered reload (any terminal
      // run status) uses this so recovering tool input/output previews from
      // the durable timeline doesn't erase a visible failure notice.
      const {
        preserveClientOnly = false,
        finalReplyTimestampByRun = null,
      } = loadOptions;
      if (!threadId) {
        setState({
          messages: [],
          messagesThreadId: null,
          nextCursor: null,
          sequenceWindow: null,
          isLoading: false,
          loadError: null,
        });
        return;
      }
      const loadKey = historyLoadKey(threadId, cursor);
      if (loadingRef.current.has(loadKey)) return;
      loadingRef.current.add(loadKey);
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
        const timelineRecords = data.messages || [];
        const fetchedSequenceWindow = timelineSequenceWindow(timelineRecords);
        const renderable = messagesFromTimeline(
          timelineRecords,
          pendingMessages,
          threadId,
        );
        const nextCursor = data.next_cursor || null;

        // RebornTimelineResponse.next_cursor === null means we reached
        // the start of the thread.
        if (!cursor) setPendingMessages?.([]);

        // A full (non-paginated) load can be cached without the previous
        // state, so refresh the cache even if the user has since switched
        // threads -- the cache write must not be deferred into `setState`,
        // which bails on a stale thread and would leave the cache stale.
        // The active thread cache is refreshed again below after merging
        // client-only messages from the live state.
        if (!cursor) {
          const cachedEntry = historyCache.get(key);
          const cachedMessages = cachedEntry?.messages || [];
          const cachedSequenceWindow =
            cachedEntry?.sequenceWindow ||
            timelineSequenceWindow(cachedMessages);
          const cacheMerged = mergeFullRefresh(renderable, cachedMessages, {
            preserveClientOnly,
            finalReplyTimestampByRun,
            freshSequenceWindow: fetchedSequenceWindow,
            currentSequenceWindow: cachedSequenceWindow,
          });
          putCache(key, {
            messages: cacheMerged,
            nextCursor: nextCursorAfterFullRefresh(
              renderable,
              cachedMessages,
              nextCursor,
              cachedEntry?.nextCursor || null,
              {
                freshSequenceWindow: fetchedSequenceWindow,
                currentSequenceWindow: cachedSequenceWindow,
              },
            ),
            sequenceWindow: sequenceWindowAfterFullRefresh(
              fetchedSequenceWindow,
              cachedSequenceWindow,
            ),
          });
        }

        setState((prev) => {
          // Stale resolve for a thread that's no longer active: leave the
          // live view alone (the cache above already captured the result).
          if (threadIdRef.current !== threadId) return prev;
          let merged;
          let mergedSequenceWindow;
          const prevSequenceWindow =
            prev.sequenceWindow || timelineSequenceWindow(prev.messages);
          if (cursor) {
            if (
              !cursorPageCanMerge(
                cursor,
                renderable,
                prev.messages,
                prev.nextCursor || null,
                {
                  pageSequenceWindow: fetchedSequenceWindow,
                  currentSequenceWindow: prevSequenceWindow,
                },
              )
            ) {
              return {
                ...prev,
                isLoading: hasOtherActiveLoadsForThread(
                  loadingRef.current,
                  threadId,
                  loadKey,
                ),
              };
            }
            merged = mergePage(renderable, prev.messages);
            mergedSequenceWindow = mergeSequenceWindows(
              fetchedSequenceWindow,
              prevSequenceWindow,
            );
          } else {
            merged = mergeFullRefresh(renderable, prev.messages, {
              preserveClientOnly,
              finalReplyTimestampByRun,
              freshSequenceWindow: fetchedSequenceWindow,
              currentSequenceWindow: prevSequenceWindow,
            });
            mergedSequenceWindow = sequenceWindowAfterFullRefresh(
              fetchedSequenceWindow,
              prevSequenceWindow,
            );
          }
          const mergedNextCursor = cursor
            ? nextCursor
            : nextCursorAfterFullRefresh(
                renderable,
                prev.messages,
                nextCursor,
                prev.nextCursor || null,
                {
                  freshSequenceWindow: fetchedSequenceWindow,
                  currentSequenceWindow: prevSequenceWindow,
                },
              );
          putCache(key, {
            messages: merged,
            nextCursor: mergedNextCursor,
            sequenceWindow: mergedSequenceWindow,
          });
          return {
            messages: merged,
            messagesThreadId: threadId,
            nextCursor: mergedNextCursor,
            sequenceWindow: mergedSequenceWindow,
            isLoading: hasOtherActiveLoadsForThread(
              loadingRef.current,
              threadId,
              loadKey,
            ),
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
                isLoading: hasOtherActiveLoadsForThread(
                  loadingRef.current,
                  threadId,
                  loadKey,
                ),
                loadError: "chat.history.loadFailed",
              }
            : s,
        );
      } finally {
        loadingRef.current.delete(loadKey);
      }
    },
    [threadId, getPendingMessages, setPendingMessages],
  );

  React.useEffect(() => {
    const entry = threadId ? historyCache.get(cacheKey(threadId)) : null;
    setState({
      messages: entry?.messages || [],
      messagesThreadId: threadId || null,
      nextCursor: entry?.nextCursor || null,
      sequenceWindow:
        entry?.sequenceWindow || timelineSequenceWindow(entry?.messages || []),
      // Only show the loading state when nothing is cached to show;
      // otherwise render the cached thread immediately and refresh in
      // the background so the content area doesn't flash empty.
      isLoading: Boolean(threadId) && !entry,
      loadError: null,
    });
    if (threadId) loadHistory();
  }, [threadId, loadHistory]);

  const seedThreadMessages = React.useCallback((targetThreadId, updater) => {
    if (!targetThreadId) return;
    const key = cacheKey(targetThreadId);
    const apply = (messages) =>
      typeof updater === "function" ? updater(messages || []) : updater;

    if (threadIdRef.current === targetThreadId) {
      setState((s) => {
        const messages = apply(s.messages || []);
        const sequenceWindow = mergeSequenceWindows(
          s.sequenceWindow || null,
          timelineSequenceWindow(messages),
        );
        putCache(key, {
          messages,
          nextCursor: s.nextCursor || null,
          sequenceWindow,
        });
        return {
          ...s,
          messages,
          sequenceWindow,
          messagesThreadId: targetThreadId,
        };
      });
      return;
    }

    const entry = historyCache.get(key) || { messages: [], nextCursor: null };
    const messages = apply(entry.messages || []);
    putCache(key, {
      messages,
      nextCursor: entry.nextCursor || null,
      sequenceWindow: mergeSequenceWindows(
        entry.sequenceWindow || null,
        timelineSequenceWindow(messages),
      ),
    });
  }, []);

  return {
    messages: state.messages,
    messagesThreadId: state.messagesThreadId,
    hasMore: Boolean(state.nextCursor),
    nextCursor: state.nextCursor,
    isLoading: state.isLoading,
    loadError: state.loadError,
    loadHistory,
    seedThreadMessages,
    setMessages: (updater) =>
      setState((s) => {
        const messages =
          typeof updater === "function" ? updater(s.messages) : updater;
        const sequenceWindow = mergeSequenceWindows(
          s.sequenceWindow || null,
          timelineSequenceWindow(messages),
        );
        // Keep the cache in step with optimistic sends and SSE-driven
        // updates so returning to the thread shows the latest messages.
        if (threadId) {
          putCache(cacheKey(threadId), {
            messages,
            nextCursor: s.nextCursor,
            sequenceWindow,
          });
        }
        return {
          ...s,
          messages,
          sequenceWindow,
          messagesThreadId: threadId || s.messagesThreadId,
        };
      }),
  };
}

function mergePage(older, current) {
  const ids = new Set(current.map((m) => m?.id).filter(Boolean));
  return [...older.filter((m) => !ids.has(m?.id)), ...current];
}

function cursorPageCanMerge(
  requestedCursor,
  pageMessages,
  currentMessages,
  currentNextCursor,
  options = {},
) {
  return (
    requestedCursor === currentNextCursor ||
    cursorPageConnectsToCurrentOldest(pageMessages, currentMessages, options)
  );
}

function cursorPageConnectsToCurrentOldest(
  pageMessages,
  currentMessages,
  options = {},
) {
  const pageWindow =
    options.pageSequenceWindow || timelineSequenceWindow(pageMessages);
  const currentWindow =
    options.currentSequenceWindow || timelineSequenceWindow(currentMessages);
  if (!pageWindow || !currentWindow) return false;
  return (
    pageWindow.oldest <= currentWindow.oldest &&
    pageWindow.newest + 1 >= currentWindow.oldest
  );
}

function historyLoadKey(threadId, cursor) {
  return `${threadId}\u0000${cursor || ""}`;
}

function hasOtherActiveLoadsForThread(activeLoadKeys, threadId, currentLoadKey) {
  const prefix = `${threadId}\u0000`;
  for (const key of activeLoadKeys) {
    if (key !== currentLoadKey && key.startsWith(prefix)) return true;
  }
  return false;
}

function nextCursorAfterFullRefresh(
  fresh,
  current,
  freshNextCursor,
  currentNextCursor,
  options = {},
) {
  if (!freshNextCursor) return null;
  const freshSequenceWindow =
    options.freshSequenceWindow || timelineSequenceWindow(fresh);
  if (!freshSequenceWindow) return freshNextCursor;
  const currentSequenceWindow =
    options.currentSequenceWindow || timelineSequenceWindow(current);
  return hasConnectedLoadedOlderTimelineRecords(
    current,
    freshSequenceWindow,
    currentSequenceWindow,
  )
    ? currentNextCursor || null
    : freshNextCursor;
}

function mergeFullRefresh(fresh, current, options = {}) {
  const {
    preserveClientOnly = false,
    finalReplyTimestampByRun = null,
    freshSequenceWindow: rawFreshSequenceWindow = null,
    currentSequenceWindow: rawCurrentSequenceWindow = null,
  } = options;
  const hydratedFresh = carryFinalAssistantOrderFlags(
    hydrateFreshMessages(fresh, current, {
      finalReplyTimestampByRun,
    }),
    current,
  );
  const ids = new Set(hydratedFresh.map((m) => m?.id).filter(Boolean));
  const freshSequenceWindow =
    rawFreshSequenceWindow || timelineSequenceWindow(hydratedFresh);
  const currentSequenceWindow =
    rawCurrentSequenceWindow || timelineSequenceWindow(current);
  const preserveLoadedOlderTimelineMessages =
    freshSequenceWindow &&
    currentTimelineWindowConnectsToFresh(
      current,
      freshSequenceWindow,
      currentSequenceWindow,
    );
  const preserved = current.filter((message) => {
    if (!message || typeof message.id !== "string" || ids.has(message.id)) {
      return false;
    }
    if (isRunActivityMessage(message) && timelineSequence(message) === null) {
      return true;
    }
    if (
      typeof message.timelineMessageId === "string" &&
      ids.has(`msg-${message.timelineMessageId}`)
    ) {
      return false;
    }
    if (isSeededOptimisticMessage(message)) return true;
    if (
      preserveLoadedOlderTimelineMessages &&
      isLoadedOlderTimelineMessage(message, freshSequenceWindow.oldest)
    ) {
      return true;
    }
    return preserveClientOnly && message.id.startsWith("err-");
  });
  return preserved.length > 0
    ? insertPreservedAtOriginalPositions(hydratedFresh, preserved, current)
    : hydratedFresh;
}

function isSeededOptimisticMessage(message) {
  return (
    message?.isOptimistic === true &&
    typeof message.id === "string" &&
    message.id.startsWith("pending-") &&
    (message.role === "user" || message.role === "assistant")
  );
}

function isLoadedOlderTimelineMessage(message, oldestFreshSequence) {
  if (oldestFreshSequence === null) return false;
  const sequence = timelineSequence(message);
  return sequence !== null && sequence < oldestFreshSequence;
}

function hasConnectedLoadedOlderTimelineRecords(
  messages,
  freshSequenceWindow,
  currentSequenceWindow,
) {
  if (
    !currentTimelineWindowConnectsToFresh(
      messages,
      freshSequenceWindow,
      currentSequenceWindow,
    )
  ) {
    return false;
  }
  if (
    currentSequenceWindow &&
    currentSequenceWindow.oldest < freshSequenceWindow.oldest
  ) {
    return true;
  }
  return (messages || []).some((message) =>
    isLoadedOlderTimelineMessage(message, freshSequenceWindow.oldest),
  );
}

function currentTimelineWindowConnectsToFresh(
  messages,
  freshSequenceWindow,
  currentSequenceWindow = null,
) {
  if (
    currentSequenceWindow &&
    sequenceWindowsOverlapOrTouch(currentSequenceWindow, freshSequenceWindow)
  ) {
    return true;
  }
  let newestBeforeFresh = null;
  for (const message of messages || []) {
    const sequence = timelineSequence(message);
    if (sequence === null) continue;
    if (
      sequence >= freshSequenceWindow.oldest &&
      sequence <= freshSequenceWindow.newest
    ) {
      return true;
    }
    if (sequence < freshSequenceWindow.oldest) {
      newestBeforeFresh =
        newestBeforeFresh === null
          ? sequence
          : Math.max(newestBeforeFresh, sequence);
    }
  }
  return (
    newestBeforeFresh !== null &&
    newestBeforeFresh + 1 === freshSequenceWindow.oldest
  );
}

function sequenceWindowAfterFullRefresh(freshSequenceWindow, currentSequenceWindow) {
  if (!freshSequenceWindow) return freshSequenceWindow;
  if (
    currentSequenceWindow &&
    sequenceWindowsOverlapOrTouch(currentSequenceWindow, freshSequenceWindow)
  ) {
    return mergeSequenceWindows(freshSequenceWindow, currentSequenceWindow);
  }
  return freshSequenceWindow;
}

function mergeSequenceWindows(left, right) {
  if (!left) return right || null;
  if (!right) return left || null;
  return {
    oldest: Math.min(left.oldest, right.oldest),
    newest: Math.max(left.newest, right.newest),
  };
}

function sequenceWindowsOverlapOrTouch(left, right) {
  if (!left || !right) return false;
  return left.oldest <= right.newest + 1 && right.oldest <= left.newest + 1;
}

function timelineSequenceWindow(messages) {
  let oldest = null;
  let newest = null;
  for (const message of messages || []) {
    const sequence = timelineSequence(message);
    if (sequence === null) continue;
    oldest = oldest === null ? sequence : Math.min(oldest, sequence);
    newest = newest === null ? sequence : Math.max(newest, sequence);
  }
  return oldest === null ? null : { oldest, newest };
}

function timelineSequence(message) {
  const sequence = Number(message?.sequence);
  return Number.isFinite(sequence) ? sequence : null;
}

function hydrateFreshMessages(fresh, current, options = {}) {
  const { finalReplyTimestampByRun = null } = options;
  const currentByConfirmedId = new Map();
  const finalAssistantByRun = new Map();
  for (const message of current || []) {
    if (!message || !message.timestamp) continue;
    if (typeof message.id === "string") {
      currentByConfirmedId.set(message.id, message);
    }
    if (typeof message.timelineMessageId === "string") {
      currentByConfirmedId.set(`msg-${message.timelineMessageId}`, message);
    }
    if (isFinalAssistantMessage(message) && typeof message.turnRunId === "string") {
      finalAssistantByRun.set(message.turnRunId, message);
    }
  }

  if (
    currentByConfirmedId.size === 0 &&
    finalAssistantByRun.size === 0 &&
    !finalReplyTimestampByRun
  ) {
    return fresh;
  }
  return fresh.map((message) => {
    if (!message || message.timestamp || typeof message.id !== "string") {
      return message;
    }
    const turnRunId = typeof message.turnRunId === "string" ? message.turnRunId : null;
    const currentMessage =
      currentByConfirmedId.get(message.id) ||
      (isFinalAssistantMessage(message) && turnRunId
        ? finalAssistantByRun.get(turnRunId)
        : null);
    const fallbackTimestamp =
      isFinalAssistantMessage(message) && turnRunId
        ? finalReplyTimestampByRun?.[turnRunId]
        : null;
    const timestamp = currentMessage?.timestamp || fallbackTimestamp;
    return timestamp
      ? { ...message, timestamp }
      : message;
  });
}

function insertPreservedAtOriginalPositions(fresh, preserved, current) {
  const base = mergeTimelineMessagesBySequence(
    fresh,
    preserved.filter((message) => timelineSequence(message) !== null),
  );
  const anchoredPreserved = preserved.filter(
    (message) => timelineSequence(message) === null,
  );
  const freshIndexById = new Map();
  for (const [index, message] of base.entries()) {
    if (typeof message?.id === "string") freshIndexById.set(message.id, index);
  }
  const currentAnchors = current.map((message) =>
    freshIndexForCurrentMessage(message, freshIndexById),
  );
  const after = new Map();
  const append = [];

  for (const message of anchoredPreserved) {
    if (!isRunActivityMessage(message)) {
      append.push(message);
      continue;
    }
    const originalIndex = current.indexOf(message);
    let previousAnchor = null;
    for (let index = originalIndex - 1; index >= 0; index -= 1) {
      if (currentAnchors[index] !== null) {
        previousAnchor = currentAnchors[index];
        break;
      }
    }
    if (previousAnchor !== null) {
      const group = after.get(previousAnchor) || [];
      group.push(message);
      after.set(previousAnchor, group);
    } else {
      append.push(message);
    }
  }

  const merged = [];
  for (const [index, message] of base.entries()) {
    merged.push(message);
    const group = after.get(index);
    if (group) merged.push(...group);
  }
  merged.push(...append);
  return merged;
}

function mergeTimelineMessagesBySequence(fresh, preserved) {
  if (preserved.length === 0) return fresh;
  return [...fresh, ...preserved].sort((left, right) => {
    const leftSequence = timelineSequence(left);
    const rightSequence = timelineSequence(right);
    if (leftSequence === null && rightSequence === null) return 0;
    if (leftSequence === null) return 1;
    if (rightSequence === null) return -1;
    return leftSequence - rightSequence;
  });
}

function freshIndexForCurrentMessage(message, freshIndexById) {
  if (!message) return null;
  if (typeof message.id === "string" && freshIndexById.has(message.id)) {
    return freshIndexById.get(message.id);
  }
  if (typeof message.timelineMessageId === "string") {
    const timelineId = `msg-${message.timelineMessageId}`;
    if (freshIndexById.has(timelineId)) return freshIndexById.get(timelineId);
  }
  return null;
}
