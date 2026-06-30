import { useQuery } from "@tanstack/react-query";
import { React } from "../lib/html.js";
import { listThreads } from "../lib/api.js";
import { useI18n } from "../lib/i18n.js";
import { THREAD_STATE } from "../lib/thread-state.js";
import {
  approvalThreadNotifications,
  getNotificationState,
  markNotificationIdsSeen,
  subscribeNotifications,
} from "../lib/notifications.js";

const NOTIFICATION_THREAD_LIMIT = 20;
const NOTIFICATION_REFETCH_MS = 10_000;

function profileScope(profile) {
  return profile?.tenant_id && profile?.user_id
    ? `${profile.tenant_id}:${profile.user_id}`
    : "anon";
}

function normalizeThread(record) {
  return {
    ...record,
    id: record?.id || record?.thread_id,
    state: record?.state || null,
    updated_at: record?.updated_at || null,
    created_at: record?.created_at || null,
  };
}

export function useNotifications({
  profile,
  enabled = true,
  activeThreadId = null,
} = {}) {
  const { t } = useI18n();
  const scope = profileScope(profile);
  const [notificationState, setNotificationState] = React.useState(() =>
    getNotificationState(scope),
  );

  React.useEffect(() => {
    setNotificationState(getNotificationState(scope));
    return subscribeNotifications((nextState, changedScope) => {
      if (changedScope === scope) setNotificationState(nextState);
    });
  }, [scope]);

  const query = useQuery({
    queryKey: ["notifications", "approval-threads", scope],
    queryFn: () =>
      listThreads({
        limit: NOTIFICATION_THREAD_LIMIT,
        needsApproval: true,
      }),
    enabled,
    refetchInterval: NOTIFICATION_REFETCH_MS,
    refetchIntervalInBackground: false,
  });

  const approvalMessages = React.useMemo(() => {
    const records = Array.isArray(query.data?.threads) ? query.data.threads : [];
    const approvalThreads = records.map((record) => ({
      ...normalizeThread(record),
      state: record?.state || THREAD_STATE.NEEDS_ATTENTION,
    }));
    return approvalThreadNotifications(approvalThreads, new Map(), t);
  }, [query.data, t]);

  const messages = React.useMemo(
    () =>
      approvalMessages.filter(
        (message) => !notificationState.seenIds.has(message.id),
      ),
    [approvalMessages, notificationState],
  );

  React.useEffect(() => {
    if (!activeThreadId) return;
    const activeMessageIds = approvalMessages
      .filter(
        (message) =>
          message.href === `/chat/${encodeURIComponent(activeThreadId)}` &&
          !notificationState.seenIds.has(message.id),
      )
      .map((message) => message.id);
    if (activeMessageIds.length === 0) return;
    const next = markNotificationIdsSeen(activeMessageIds, scope);
    setNotificationState(next);
  }, [activeThreadId, approvalMessages, notificationState, scope]);

  const unreadIds = React.useMemo(
    () => new Set(messages.map((message) => message.id)),
    [messages],
  );

  const dismissMessage = React.useCallback((messageId) => {
    const next = markNotificationIdsSeen([messageId], scope);
    setNotificationState(next);
  }, [scope]);

  return {
    messages,
    unreadIds,
    unreadCount: unreadIds.size,
    hasUnread: unreadIds.size > 0,
    isLoading: query.isLoading,
    error: query.error || null,
    refetch: query.refetch,
    dismissMessage,
  };
}
