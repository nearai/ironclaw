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
/* A stop so one hook cannot walk an unbounded inbox in a single pass. The
 * store's own per-recipient bound is far larger than anyone pages through by
 * hand, and the control retires before this whenever the surface stops
 * reporting a cursor. */
const NOTIFICATION_PAGE_MAX = 20;

const NOTIFICATION_THREAD_LIMIT = 20;
const NOTIFICATION_REFETCH_MS = 10_000;

/* Read the head plus every page the reader has asked to keep, following the
 * cursor the surface reports. The pages come back as one flat list so the
 * optimistic mark-read and archive writes, the unread total and the
 * compatibility de-duplication all keep working on a single shape — and so a
 * poll refreshes every loaded page instead of leaving appended ones to rot. */
async function readInboxPages(pages, signal) {
  const head = await listNotifications({ limit: NOTIFICATION_LIMIT, signal });
  const notifications = [...(head?.notifications || [])];
  let cursor = head?.next_cursor || null;
  for (let page = 1; page < pages && cursor; page += 1) {
    /* Every page is a separate request, so an unmount or a superseding refetch
     * would otherwise keep walking the cursor to the end of the inbox. */
    if (signal?.aborted) throw signal.reason ?? new Error("aborted");
    const next = await listNotifications({
      limit: NOTIFICATION_LIMIT,
      cursor,
      signal,
    });
    notifications.push(...(next?.notifications || []));
    cursor = next?.next_cursor || null;
  }
  return {
    ...head,
    notifications,
    // The surface counts unread across the whole inbox, not per page, so the
    // head's total is already the real one.
    next_cursor: cursor,
  };
}

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
      const previous = queryClient.getQueryData(queryKey);
      queryClient.setQueryData(queryKey, (value) =>
        inboxCacheAfter(value, notificationId, archive),
      );
      return { previous };
    },
    onError: (_error, _notificationId, context) => {
      if (context?.previous) queryClient.setQueryData(queryKey, context.previous);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey }),
  };
}

export function useNotifications(
  options: { profile?: any; enabled?: boolean } = {},
) {
  const { profile, enabled = true } = options;
  const { t } = useI18n();
  const queryClient = /** @type {any} */ (useQueryClient());
  const threadStates = useThreadStates();
  const [pendingRenderedNotification, setPendingRenderedNotification] = React.useState(
    /** @type {{ notificationId: string, threadId: string, turnRunId: string } | null} */ (null),
  );
  const tenantId = profile?.tenant_id || null;
  const userId = profile?.user_id || null;
  const scope = tenantId && userId ? `${tenantId}:${userId}` : null;
  /* How many pages the reader has asked to keep loaded. The polled query owns
   * them all, so there is no second list to fall out of step with the head. */
  const [loadedPages, setLoadedPages] = React.useState(1);
  /* The page count is a request parameter, not an identity, so it stays out of
   * the key. Keying on it splits the cache: "load more" would land on an entry
   * with no data yet, blanking the open panel and clearing the badge until the
   * refetch returned, and every optimistic write and invalidation below would
   * reach only the current page count while entries under the others kept
   * stale read/unread state. One entry, refetched when the count changes. */
  const queryKey = React.useMemo(
    () => ["notifications", "inbox", tenantId, userId],
    [tenantId, userId],
  );

  const query = useQuery({
    queryKey,
    queryFn: async ({ signal }) => {
      /* Read the inbox first and fall back only where it does not exist. These
       * used to run as a pair on every poll, which cost a discarded
       * approval-thread query — and a dynamic import of the presenter — every
       * ten seconds in every signed-in tab, for a result the surface throws
       * away as soon as the inbox answers. The fallback surface pays one extra
       * round-trip instead; it is the surface that has no inbox at all. */
      let inbox;
      try {
        inbox = await readInboxPages(loadedPages, signal);
      } catch (error) {
        if (!isNotificationInboxUnsupported(error)) throw error;
        const approvalThreads = await listThreads({
          limit: NOTIFICATION_THREAD_LIMIT,
          needsApproval: true,
          signal,
        });
        const presenter = await import("../lib/notification-approval-compat");
        const seenIds = presenter.getNotificationState(scope).seenIds;
        const records = Array.isArray(approvalThreads?.threads)
          ? approvalThreads.threads
          : [];
        return {
          inbox: { notifications: [], unread_count: 0 },
          inboxSupported: false,
          compatibility: presenter
            .approvalThreadNotifications(records.map(normalizeThread), threadStates, t)
            .map((message) => ({
              ...message,
              durable: false,
              read: seenIds.has(message.id),
            })),
        };
      }
      return { inbox, inboxSupported: true, compatibility: [] };
    },
    enabled: enabled && Boolean(tenantId && userId),
    refetchInterval: NOTIFICATION_REFETCH_MS,
    refetchIntervalInBackground: false,
  });

  const inboxSupported = query.data?.inboxSupported !== false;
  const messages = React.useMemo(() => {
    const durable = notificationMessages(query.data?.inbox?.notifications, t).map(
      (message) => ({ ...message, durable: true }),
    );
    /* The compatibility rows stand in for a surface that cannot serve the
     * durable inbox at all. Mixing them in alongside durable records looked
     * safer, but suppression could only be computed from the rows currently
     * loaded: archiving a durable approval dropped it from that set and the
     * pending-thread row for the same thread came straight back, unread and
     * with no archive control. Where the inbox answers, durable records are the
     * only truth — including the truth that one was archived. */
    const compatibility = inboxSupported
      ? []
      : (query.data?.compatibility || []).map((message) => ({
          ...message,
          durable: false,
        }));
    return [...durable, ...compatibility].sort(
      (left, right) => right.timestamp - left.timestamp,
    );
  }, [inboxSupported, query.data, t]);
  const unreadIds = React.useMemo(
    () => new Set(messages.filter((message) => !message.read).map((message) => message.id)),
    [messages],
  );

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

  // `next_cursor` is the surface's own has-more signal, and it now survives
  // paging: the merged result carries the last loaded page's cursor.
  const hasMorePages = Boolean(query.data?.inbox?.next_cursor);
  const canLoadMore = hasMorePages && loadedPages < NOTIFICATION_PAGE_MAX;
  /* Records remain past the reader's own ceiling. Hiding the control on its own
   * reads as "that is everything", which is the one thing it does not mean. */
  const pageLimitReached = hasMorePages && loadedPages >= NOTIFICATION_PAGE_MAX;
  const loadMore = React.useCallback(() => {
    setLoadedPages((current) => Math.min(current + 1, NOTIFICATION_PAGE_MAX));
  }, []);
  /* With the count out of the key, a bump changes no identity React Query
   * watches, so ask for the wider read explicitly. The rows already on screen
   * stay put while it runs. */
  const refetch = query.refetch;
  React.useEffect(() => {
    if (loadedPages > 1) refetch?.();
  }, [loadedPages, refetch]);

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
    pageLimitReached,
    loadedPages,
  };
}
