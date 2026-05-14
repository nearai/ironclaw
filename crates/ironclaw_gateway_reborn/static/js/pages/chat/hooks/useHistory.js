import { React } from "../../../lib/html.js";
import { fetchHistory } from "../../../lib/api.js";
import {
  appendInProgressMessage,
  turnsToMessages,
} from "../lib/history-messages.js";

export function useHistory(threadId, options = {}) {
  const { getPendingMessages, setPendingMessages } = options;
  const [state, setState] = React.useState({
    messages: [],
    hasMore: false,
    oldestTimestamp: null,
    isLoading: false,
    inProgress: null,
    pendingGate: null,
  });

  const loadHistory = React.useCallback(
    async (before) => {
      if (!threadId) {
        setState({
          messages: [],
          hasMore: false,
          oldestTimestamp: null,
          isLoading: false,
          inProgress: null,
          pendingGate: null,
        });
        return;
      }
      setState((s) => ({ ...s, isLoading: true }));
      try {
        const data = await fetchHistory({ threadId, limit: 50, before });
        const pendingMessages = before ? [] : getPendingMessages?.() || [];
        const result = turnsToMessages(data.turns || [], {
          threadId,
          pendingMessages,
        });
        const newMessages = before
          ? result.messages
          : appendInProgressMessage(
              result.messages,
              data.in_progress,
              result.remainingPending
            );

        if (!before) {
          setPendingMessages?.(result.remainingPending);
        }

        setState((prev) => {
          const existingIds = new Set(prev.messages.map((m) => m.id));
          const deduped = before
            ? newMessages.filter((m) => !existingIds.has(m.id))
            : newMessages;
          return {
            messages: before ? [...deduped, ...prev.messages] : deduped,
            hasMore: data.has_more,
            oldestTimestamp: data.oldest_timestamp,
            isLoading: false,
            inProgress: data.in_progress || null,
            pendingGate: data.pending_gate || null,
          };
        });
      } catch (err) {
        setState((s) => ({ ...s, isLoading: false }));
        console.error("Failed to load history:", err);
      }
    },
    [threadId, getPendingMessages, setPendingMessages]
  );

  React.useEffect(() => {
    setState({
      messages: [],
      hasMore: false,
      oldestTimestamp: null,
      isLoading: Boolean(threadId),
      inProgress: null,
      pendingGate: null,
    });
    if (threadId) {
      loadHistory();
    }
  }, [threadId, loadHistory]);

  return {
    messages: state.messages,
    hasMore: state.hasMore,
    oldestTimestamp: state.oldestTimestamp,
    isLoading: state.isLoading,
    inProgress: state.inProgress,
    pendingGate: state.pendingGate,
    loadHistory,
    setMessages: (updater) =>
      setState((s) => ({
        ...s,
        messages: typeof updater === "function" ? updater(s.messages) : updater,
      })),
  };
}
