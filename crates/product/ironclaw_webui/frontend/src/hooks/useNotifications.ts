// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import {
  listThreads,
  listNotifications,
  markAllNotificationsRead,
  markNotificationRead,
} from "../lib/api";
import { useI18n } from "../lib/i18n";
import { useThreadStates } from "../lib/thread-state";
import { notificationMessages } from "../lib/notifications";

const NOTIFICATION_LIMIT = 30;
const NOTIFICATION_THREAD_LIMIT = 20;
const NOTIFICATION_REFETCH_MS = 10_000;

function isNotificationInboxUnsupported(error) {
  const status = Number(error?.status);
  return status === 404 || status === 405 || status === 501;
}

function normalizeThread(record) {
  return {
    ...record,
    id: record?.id || record?.thread_id,
    state: record?.state || "needs_attention",
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
  const queryClient = useQueryClient();
  const threadStates = useThreadStates();
  const tenantId = profile?.tenant_id || null;
  const userId = profile?.user_id || null;
  const scope = tenantId && userId ? `${tenantId}:${userId}` : null;
  const queryKey = ["notifications", "inbox", tenantId, userId];

  const query = useQuery({
    queryKey,
    queryFn: async () => {
      // During the durable-inbox rollout, approval producers may land after
      // this consumer. Read both sources so switching the UI does not make
      // existing approval notifications disappear. Durable records win in
      // the presentation-layer de-duplication below.
      const [inboxResult, approvalResult] = await Promise.allSettled([
        listNotifications({ limit: NOTIFICATION_LIMIT }),
        listThreads({
          limit: NOTIFICATION_THREAD_LIMIT,
          needsApproval: true,
        }),
      ]);

      let inboxSupported = true;
      let inbox;
      if (inboxResult.status === "fulfilled") {
        inbox = inboxResult.value;
      } else if (isNotificationInboxUnsupported(inboxResult.reason)) {
        inboxSupported = false;
        inbox = { notifications: [], unread_count: 0 };
      } else {
        throw inboxResult.reason;
      }

      if (approvalResult.status === "rejected") {
        // The legacy path is supplemental once the durable inbox exists. Do
        // not hide durable notifications because the compatibility read
        // failed; without an inbox, however, there is no usable data source.
        if (!inboxSupported) throw approvalResult.reason;
        return { inbox, inboxSupported, compatibility: [] };
      }

      const approvalThreads = approvalResult.value;
      const presenter = await import("../lib/notification-approval-compat");
      const seenIds = presenter.getNotificationState(scope).seenIds;
      const records = Array.isArray(approvalThreads?.threads)
        ? approvalThreads.threads
        : [];
      const compatibility = presenter
        .approvalThreadNotifications(records.map(normalizeThread), threadStates, t)
        .map((message) => ({
          ...message,
          durable: false,
          read: seenIds.has(message.id),
        }));
      return {
        inbox,
        inboxSupported,
        compatibility,
      };
    },
    enabled: enabled && Boolean(tenantId && userId),
    refetchInterval: NOTIFICATION_REFETCH_MS,
    refetchIntervalInBackground: false,
  });

  const messages = React.useMemo(() => {
    const durable = notificationMessages(query.data?.inbox?.notifications, t).map(
      (message) => ({ ...message, durable: true }),
    );
    const durableThreadHrefs = new Set(
      durable
        .filter((message) => message.type === "approval_required" && message.href)
        .map((message) => message.href),
    );
    const compatibility = (query.data?.compatibility || [])
      .filter((message) => !durableThreadHrefs.has(message.href))
      .map((message) => ({ ...message, durable: false }));
    return [...durable, ...compatibility].sort(
      (left, right) => right.timestamp - left.timestamp,
    );
  }, [query.data, t]);
  const unreadIds = React.useMemo(
    () => new Set(messages.filter((message) => !message.read).map((message) => message.id)),
    [messages],
  );
  const inboxSupported = query.data?.inboxSupported !== false;

  const markRead = useMutation({
    mutationFn: markNotificationRead,
    onMutate: async (notificationId) => {
      await queryClient.cancelQueries({ queryKey });
      const previous = queryClient.getQueryData(queryKey);
      queryClient.setQueryData(queryKey, (current) => ({
        ...current,
        inbox: {
          ...current?.inbox,
          unread_count: Math.max(0, Number(current?.inbox?.unread_count || 0) - 1),
          notifications: (current?.inbox?.notifications || []).map((notification) =>
            notification.id === notificationId && !notification.read_at
              ? { ...notification, read_at: new Date().toISOString() }
              : notification,
          ),
        },
      }));
      return { previous };
    },
    onError: (_error, _notificationId, context) => {
      if (context?.previous) queryClient.setQueryData(queryKey, context.previous);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey }),
  });

  const markAllReadMutation = useMutation({
    mutationFn: markAllNotificationsRead,
    onSuccess: () => queryClient.invalidateQueries({ queryKey }),
  });

  const markCompatibilitySeen = React.useCallback(
    async (ids) => {
      if (!scope || ids.length === 0) return;
      const compatibility = await import("../lib/notification-approval-compat");
      compatibility.markNotificationIdsSeen(ids, scope);
      await queryClient.invalidateQueries({ queryKey });
    },
    [queryClient, queryKey, scope],
  );

  React.useEffect(() => {
    if (!activeThreadId) return;
    for (const message of messages) {
      if (
        !message.read &&
        message.href === `/chat/${encodeURIComponent(activeThreadId)}` &&
        !markRead.isPending
      ) {
        if (message.durable) {
          markRead.mutate(message.id);
        } else if (scope) {
          void markCompatibilitySeen([message.id]);
        }
        break;
      }
    }
  }, [activeThreadId, markCompatibilitySeen, markRead, messages, scope]);

  const dismissMessage = React.useCallback(
    (messageId) => {
      if (!unreadIds.has(messageId)) return;
      const message = messages.find((candidate) => candidate.id === messageId);
      if (message?.durable) {
        markRead.mutate(messageId);
      } else if (scope) {
        void markCompatibilitySeen([messageId]);
      }
    },
    [markCompatibilitySeen, markRead, messages, scope, unreadIds],
  );

  const markAllRead = React.useCallback(() => {
    if (scope) {
      const compatibilityIds = messages
        .filter((message) => !message.durable && !message.read)
        .map((message) => message.id);
      if (compatibilityIds.length > 0) {
        void markCompatibilitySeen(compatibilityIds);
      }
    }
    if (inboxSupported) {
      markAllReadMutation.mutate();
    }
  }, [inboxSupported, markAllReadMutation, markCompatibilitySeen, messages, scope]);

  const serverUnreadCount = Number(query.data?.inbox?.unread_count || 0);
  const compatibilityUnreadCount = messages.filter(
    (message) => !message.durable && !message.read,
  ).length;
  const unreadCount = serverUnreadCount + compatibilityUnreadCount;

  return {
    messages,
    unreadIds,
    unreadCount,
    hasUnread: unreadCount > 0,
    isLoading: query.isLoading,
    error: query.error || markRead.error || markAllReadMutation.error || null,
    refetch: query.refetch,
    dismissMessage,
    markAllRead,
    isMarkingAllRead: markAllReadMutation.isPending,
  };
}
