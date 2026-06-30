import { useQuery } from "@tanstack/react-query";
import { React } from "../lib/html.js";
import { listAutomations } from "../lib/api.js";
import { useI18n } from "../lib/i18n.js";
import { normalizeAutomations } from "../pages/automations/lib/automations-presenters.js";
import { AUTOMATIONS_BASE_REFETCH_MS } from "../pages/automations/lib/automations-refresh.js";
import {
  automationRunNotifications,
  ensureNotificationBaseline,
  getNotificationState,
  markNotificationIdsSeen,
  subscribeNotifications,
} from "../lib/notifications.js";

const NOTIFICATION_AUTOMATION_LIMIT = 25;
const NOTIFICATION_RUN_LIMIT = 10;

function profileScope(profile) {
  return profile?.tenant_id && profile?.user_id
    ? `${profile.tenant_id}:${profile.user_id}`
    : "anon";
}

export function useNotifications({ profile, enabled = true } = {}) {
  const { t, lang } = useI18n();
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
    queryKey: ["notifications", "automations", scope],
    queryFn: () =>
      listAutomations({
        limit: NOTIFICATION_AUTOMATION_LIMIT,
        runLimit: NOTIFICATION_RUN_LIMIT,
        includeCompleted: false,
      }),
    enabled,
    refetchInterval: AUTOMATIONS_BASE_REFETCH_MS,
    refetchIntervalInBackground: false,
  });

  const messages = React.useMemo(() => {
    const automations = normalizeAutomations(query.data, t, lang);
    return automationRunNotifications(automations, t);
  }, [query.data, t, lang]);

  const messageIds = React.useMemo(
    () => messages.map((message) => message.id),
    [messages],
  );

  React.useEffect(() => {
    if (!query.isSuccess) return;
    const next = ensureNotificationBaseline(messageIds, scope);
    setNotificationState(next);
  }, [messageIds, scope, query.isSuccess]);

  const unreadIds = React.useMemo(() => {
    if (!notificationState.initialized) return new Set();
    return new Set(
      messages
        .filter((message) => !notificationState.seenIds.has(message.id))
        .map((message) => message.id),
    );
  }, [messages, notificationState]);

  const markAllRead = React.useCallback(() => {
    const next = markNotificationIdsSeen(messageIds, scope);
    setNotificationState(next);
  }, [messageIds, scope]);

  return {
    messages,
    unreadIds,
    unreadCount: unreadIds.size,
    hasUnread: unreadIds.size > 0,
    isLoading: query.isLoading,
    error: query.error || null,
    refetch: query.refetch,
    markAllRead,
  };
}
