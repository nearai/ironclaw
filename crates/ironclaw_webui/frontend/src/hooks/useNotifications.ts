// @ts-nocheck
import { useQuery } from "@tanstack/react-query";
import React from "react";
import { listThreads } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { THREAD_STATE, useThreadStates } from "../lib/thread-state";
import {
  approvalThreadNotifications,
  getNotificationState,
  markNotificationIdsSeen,
  subscribeNotifications,
} from "../lib/notifications";

const NOTIFICATION_THREAD_LIMIT = 20;
const NOTIFICATION_REFETCH_MS = 10_000;

function emptyNotificationState() {
  return { initialized: false, seenIds: new Set() };
}

function profileScope(profile) {
  return profile?.tenant_id && profile?.user_id
    ? `${profile.tenant_id}:${profile.user_id}`
    : null;
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
  const threadStates = useThreadStates();
  const scope = profileScope(profile);
  const [notificationState, setNotificationState] = React.useState(() =>
    scope ? getNotificationState(scope) : emptyNotificationState(),
  );

  React.useEffect(() => {
    if (!scope) {
      setNotificationState(emptyNotificationState());
      return undefined;
    }
    setNotificationState(getNotificationState(scope));
    return subscribeNotifications((nextState, changedScope) => {
      if (changedScope === scope) setNotificationState(nextState);
    });
  }, [scope]);

  const query = useQuery({
    queryKey: ["notifications", "approval-threads", scope || "pending-profile"],
    queryFn: () =>
      listThreads({
        limit: NOTIFICATION_THREAD_LIMIT,
        needsApproval: true,
      }),
    enabled: enabled && Boolean(scope),
    refetchInterval: NOTIFICATION_REFETCH_MS,
    refetchIntervalInBackground: false,
  });

  const messages = React.useMemo(() => {
    if (!scope) return [];
    const records = Array.isArray(query.data?.threads) ? query.data.threads : [];
    const approvalThreads = records.map((record) => ({
      ...normalizeThread(record),
      state: record?.state || THREAD_STATE.NEEDS_ATTENTION,
    }));
    return approvalThreadNotifications(approvalThreads, threadStates, t);
  }, [query.data, scope, t, threadStates]);

  React.useEffect(() => {
    if (!activeThreadId || !scope) return;
    const activeMessageIds = messages
      .filter(
        (message) =>
          message.href === `/chat/${encodeURIComponent(activeThreadId)}` &&
          !notificationState.seenIds.has(message.id),
      )
      .map((message) => message.id);
    if (activeMessageIds.length === 0) return;
    const next = markNotificationIdsSeen(activeMessageIds, scope);
    setNotificationState(next);
  }, [activeThreadId, messages, notificationState, scope]);

  const unreadIds = React.useMemo(
    () =>
      new Set(
        messages
          .filter((message) => !notificationState.seenIds.has(message.id))
          .map((message) => message.id),
      ),
    [messages, notificationState],
  );

  const dismissMessage = React.useCallback((messageId) => {
    if (!scope) return;
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
