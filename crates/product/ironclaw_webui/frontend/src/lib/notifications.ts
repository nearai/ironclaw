
const PRESENTATION = {
  approval_required: { icon: "shield", key: "approval" },
  authentication_required: { icon: "key", key: "authentication" },
  run_blocked: { icon: "alert", key: "blocked" },
  run_failed: { icon: "error", key: "failed" },
  run_completed: { icon: "check", key: "completed" },
  delivery_failed: { icon: "send", key: "deliveryFailed" },
} as const;

type NotificationKind = keyof typeof PRESENTATION;

export interface NotificationWire {
  id?: string;
  kind?: string;
  created_at?: string;
  resolved_at?: string | null;
  thread_id?: string | null;
  turn_run_id?: string | null;
  read_at?: string | null;
  action?: {
    kind?: string;
    thread_id?: string;
  } | null;
}

export interface NotificationMessage {
  id: string;
  type: string;
  icon: string;
  title: string;
  body: string;
  detail: string;
  timeLabel: string;
  timestamp: number;
  href: string | null;
  threadId: string | null;
  turnRunId: string | null;
  read: boolean;
  resolved: boolean;
}

type Translate = (key: string) => string;

function presentationFor(kind: unknown): { icon: string; key: string } {
  if (typeof kind === "string" && kind in PRESENTATION) {
    return PRESENTATION[kind as NotificationKind];
  }
  return { icon: "bell", key: "generic" };
}

function timestamp(value: unknown): number {
  const parsed = value ? Date.parse(String(value)) : NaN;
  return Number.isFinite(parsed) ? parsed : 0;
}

function notificationHref(notification: NotificationWire): string | null {
  const action = notification?.action;
  if (action?.kind !== "open_thread" || !action.thread_id) return null;
  return `/chat/${encodeURIComponent(action.thread_id)}`;
}

export function notificationMessages(
  notifications: readonly NotificationWire[] | unknown = [],
  t: Translate | unknown = (key: string) => key,
): NotificationMessage[] {
  const tx: Translate = typeof t === "function" ? (t as Translate) : (key) => key;
  return (Array.isArray(notifications) ? (notifications as NotificationWire[]) : [])
    .map((notification) => {
      const presentation = presentationFor(notification?.kind);
      const createdAt = timestamp(notification?.created_at);
      return {
        id: notification?.id || "",
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
