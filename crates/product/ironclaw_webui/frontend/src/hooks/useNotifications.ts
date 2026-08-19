import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import {
  archiveNotification,
  listThreads,
  listNotifications,
  markAllNotificationsRead,
  markNotificationRead,
} from "../lib/api";
import { useI18n } from "../lib/i18n";
import { useThreadStates } from "../lib/thread-state";
import { notificationMessages } from "../lib/notifications";

const NOTIFICATION_LIMIT = 30;
/* The view's own page ceiling (`NOTIFICATION_PAGE_LIMIT_MAX` server-side).
 * Asking for more is rejected, so the control retires at this point rather
 * than sending a request the surface will refuse. */
const NOTIFICATION_LIMIT_MAX = 100;
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

function notificationOptions(options) {
  return options;
}

function notificationQueryData(value) {
  return value;
}

/* Optimistic cache transform shared by mark-read and archive: a read record
 * stays in the list, an archived one leaves it, and both drop out of the badge
 * only when they were still unread — so a repeated or concurrent call cannot
 * drive the count below the real one. */
function inboxCacheAfter(current, notificationId, archive) {
  const notifications = current?.inbox?.notifications || [];
  const unreadCount = Number(current?.inbox?.unread_count || 0);
  const wasUnread = notifications.some(
    (notification) => notification.id === notificationId && !notification.read_at,
  );
  return {
    ...current,
    inbox: {
      ...current?.inbox,
      unread_count: wasUnread ? Math.max(0, unreadCount - 1) : unreadCount,
      notifications: archive
        ? notifications.filter((notification) => notification.id !== notificationId)
        : notifications.map((notification) =>
            notification.id === notificationId && !notification.read_at
              ? { ...notification, read_at: new Date().toISOString() }
              : notification,
          ),
    },
  };
}

function optimisticHandlers(queryClient, queryKey, archive) {
  return {
    onMutate: async (notificationId) => {
      await queryClient.cancelQueries({ queryKey });
      const previous = notificationQueryData(queryClient.getQueryData(queryKey));
      queryClient.setQueryData(queryKey, (value) =>
        inboxCacheAfter(notificationQueryData(value), notificationId, archive),
      );
      return { previous };
    },
    onError: (_error, _notificationId, context) => {
      if (context?.previous) queryClient.setQueryData(queryKey, context.previous);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey }),
  };
}

export function useNotifications(options = {}) {
  const { profile, enabled = true } = notificationOptions(options);
  const { t } = useI18n();
  const queryClient = /** @type {any} */ (useQueryClient());
  const threadStates = useThreadStates();
  const [pendingRenderedNotification, setPendingRenderedNotification] = React.useState(
    /** @type {{ notificationId: string, threadId: string, turnRunId: string } | null} */ (null),
  );
  const tenantId = profile?.tenant_id || null;
  const userId = profile?.user_id || null;
  const scope = tenantId && userId ? `${tenantId}:${userId}` : null;
  /* Paging grows the page the polled query asks for instead of chaining
   * cursors into a second list. One request stays the single source of truth,
   * so the optimistic mark-read/archive writes keep applying and a poll can
   * never disagree with an appended page about the same record. */
  const [requestedLimit, setRequestedLimit] = React.useState(NOTIFICATION_LIMIT);
  const queryKey = React.useMemo(
    () => ["notifications", "inbox", tenantId, userId, requestedLimit],
    [requestedLimit, tenantId, userId],
  );

  const query = useQuery({
    queryKey,
    queryFn: async () => {
      // During the durable-inbox rollout, approval producers may land after
      // this consumer. Read both sources so switching the UI does not make
      // existing approval notifications disappear. Durable records win in
      // the presentation-layer de-duplication below.
      const [inboxResult, approvalResult] = await Promise.allSettled([
        listNotifications({ limit: requestedLimit }),
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
    ...optimisticHandlers(queryClient, queryKey, false),
  });

  const markAllReadMutation = useMutation({
    mutationFn: markAllNotificationsRead,
    onSuccess: () => queryClient.invalidateQueries({ queryKey }),
  });

  const archiveMutation = useMutation({
    mutationFn: archiveNotification,
    ...optimisticHandlers(queryClient, queryKey, true),
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

  const archiveMessage = React.useCallback(
    (messageId) => {
      // Only durable records exist server-side. The compatibility rows are
      // derived from threads needing approval, so there is nothing to archive
      // and the request would 404.
      const message = messages.find((candidate) => candidate.id === messageId);
      if (!message?.durable) return;
      archiveMutation.mutate(messageId);
    },
    [archiveMutation, messages],
  );

  const prepareMessageOpen = React.useCallback(
    (message) => {
      if (!message?.id) return;
      if (
        message.durable &&
        message.type === "run_completed" &&
        message.threadId &&
        message.turnRunId
      ) {
        setPendingRenderedNotification({
          notificationId: message.id,
          threadId: message.threadId,
          turnRunId: message.turnRunId,
        });
        return;
      }
      setPendingRenderedNotification(null);
      dismissMessage(message.id);
    },
    [dismissMessage],
  );

  const acknowledgeRenderedNotification = React.useCallback(
    ({ threadId, turnRunId }) => {
      const pending = pendingRenderedNotification;
      // No `markRead.isPending` guard: the final reply renders once per run,
      // so skipping here would strand the completion notification unread with
      // nothing left to re-trigger it. Clearing `pending` below already stops
      // a second acknowledgement for the same record.
      if (!pending || pending.threadId !== threadId || pending.turnRunId !== turnRunId) {
        return;
      }
      setPendingRenderedNotification(null);
      markRead.mutate(pending.notificationId);
    },
    [markRead, pendingRenderedNotification],
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

  // `next_cursor` is the surface's own has-more signal.
  const canLoadMore =
    Boolean(query.data?.inbox?.next_cursor) && requestedLimit < NOTIFICATION_LIMIT_MAX;
  const loadMore = React.useCallback(() => {
    setRequestedLimit((current) =>
      Math.min(current + NOTIFICATION_LIMIT, NOTIFICATION_LIMIT_MAX),
    );
  }, []);

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
    error:
      query.error ||
      markRead.error ||
      markAllReadMutation.error ||
      archiveMutation.error ||
      null,
    refetch: query.refetch,
    dismissMessage,
    prepareMessageOpen,
    pendingRenderedNotification,
    acknowledgeRenderedNotification,
    markAllRead,
    isMarkingAllRead: markAllReadMutation.isPending,
    archiveMessage,
    canLoadMore,
    loadMore,
    requestedLimit,
  };
}
