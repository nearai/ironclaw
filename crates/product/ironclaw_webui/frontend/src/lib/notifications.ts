// @ts-nocheck

const PRESENTATION = {
  approval_required: { icon: "shield", key: "approval" },
  authentication_required: { icon: "key", key: "authentication" },
  run_blocked: { icon: "alert", key: "blocked" },
  run_failed: { icon: "error", key: "failed" },
  run_completed: { icon: "check", key: "completed" },
  delivery_failed: { icon: "send", key: "deliveryFailed" },
};

function timestamp(value) {
  const parsed = value ? Date.parse(value) : NaN;
  return Number.isFinite(parsed) ? parsed : 0;
}

function notificationHref(notification) {
  const action = notification?.action;
  if (action?.kind !== "open_thread" || !action.thread_id) return null;
  return `/chat/${encodeURIComponent(action.thread_id)}`;
}

export function notificationMessages(notifications = [], t = (key) => key) {
  const tx = typeof t === "function" ? t : (key) => key;
  return (Array.isArray(notifications) ? notifications : [])
    .map((notification) => {
      const presentation = PRESENTATION[notification?.kind] || {
        icon: "bell",
        key: "generic",
      };
      const createdAt = timestamp(notification?.created_at);
      return {
        id: notification?.id,
        type: notification?.kind || "generic",
        icon: presentation.icon,
        title: tx(`notifications.${presentation.key}.title`),
        body: tx(`notifications.${presentation.key}.body`),
        detail: notification?.resolved_at
          ? tx("notifications.resolved")
          : tx(`notifications.${presentation.key}.detail`),
        timeLabel: createdAt
          ? new Date(createdAt).toLocaleString([], {
              month: "short",
              day: "numeric",
              hour: "2-digit",
              minute: "2-digit",
            })
          : "",
        timestamp: createdAt,
        href: notificationHref(notification),
        threadId: notification?.thread_id || notification?.action?.thread_id || null,
        turnRunId: notification?.turn_run_id || null,
        read: Boolean(notification?.read_at),
        resolved: Boolean(notification?.resolved_at),
      };
    })
    .filter((message) => Boolean(message.id))
    .sort((left, right) => right.timestamp - left.timestamp);
}
