import { React } from "../../../lib/html.js";
import { fetchTimeline } from "../../../lib/api.js";
import { messagesFromTimeline } from "../lib/history-messages.js";

const PAGE_SIZE = 50;

export function useHistory(threadId, options = {}) {
  const { getPendingMessages, setPendingMessages } = options;
  const [state, setState] = React.useState({
    messages: [],
    nextCursor: null,
    isLoading: false,
  });

  const loadHistory = React.useCallback(
    async (cursor) => {
      if (!threadId) {
        setState({ messages: [], nextCursor: null, isLoading: false });
        return;
      }
      setState((s) => ({ ...s, isLoading: true }));
      try {
        const data = await fetchTimeline({
          threadId,
          limit: PAGE_SIZE,
          cursor,
        });

        const pendingMessages = cursor ? [] : getPendingMessages?.() || [];
        const renderable = messagesFromTimeline(data.messages || [], pendingMessages);

        // RebornTimelineResponse.next_cursor === null means we reached
        // the start of the thread.
        if (!cursor) setPendingMessages?.([]);

        setState((prev) => {
          const merged = cursor
            ? mergePage(renderable, prev.messages)
            : renderable;
          return {
            messages: merged,
            nextCursor: data.next_cursor || null,
            isLoading: false,
          };
        });
      } catch (err) {
        setState((s) => ({ ...s, isLoading: false }));
        // Stay loud — surface to the SPA error boundary rather than
        // silently masking timeline outages.
        console.error("Failed to load timeline:", err);
      }
    },
    [threadId, getPendingMessages, setPendingMessages],
  );

  React.useEffect(() => {
    setState({
      messages: [],
      nextCursor: null,
      isLoading: Boolean(threadId),
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
      setState((s) => ({
        ...s,
        messages: typeof updater === "function" ? updater(s.messages) : updater,
      })),
  };
}

function mergePage(older, current) {
  const ids = new Set(current.map((m) => m.id));
  return [...older.filter((m) => !ids.has(m.id)), ...current];
}
