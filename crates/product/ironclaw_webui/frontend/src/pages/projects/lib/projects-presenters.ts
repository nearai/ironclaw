import { interpolateParams } from "../../../lib/i18n-format";

function tx(t, key, params = {}, fallback = key) {
  return typeof t === "function" ? t(key, params) : interpolateParams(fallback, params);
}

function formatEnumLabel(value, t, config) {
  const { labels, keyPrefix, defaultKey = "unknown", translationKeys = {}, unknownLabel } = config;
  const key = String(value || defaultKey).toLowerCase();
  const fallback = labels[key];

  if (!fallback) {
    return unknownLabel ? unknownLabel(value, labels[defaultKey]) : String(value || labels[defaultKey]);
  }

  return tx(t, translationKeys[key] || `${keyPrefix}.${key}`, {}, fallback);
}

const PROJECT_HEALTH_LABELS = {
  green: "Healthy",
  yellow: "Needs review",
  red: "At risk",
  muted: "Archived",
  steady: "Steady",
  unknown: "Unknown",
};

const PROJECT_STATE_LABELS = {
  active: "Active",
  archived: "Archived",
  unknown: "Unknown",
};

const PROJECT_ROLE_LABELS = {
  owner: "Owner",
  editor: "Editor",
  viewer: "Viewer",
  unknown: "Unknown",
};

const THREAD_STATE_LABELS = {
  running: "Running",
  done: "Done",
  completed: "Completed",
  failed: "Failed",
  unknown: "Unknown",
};

const THREAD_TYPE_LABELS = {
  mission_run: "Mission run",
};

const MESSAGE_ROLE_LABELS = {
  system: "System",
  user: "User",
  assistant: "Assistant",
  tool: "Tool",
};

export function formatProjectDate(iso, t, options = {}) {
  if (!iso) return tx(t, "projects.date.notAvailable", {}, "Not available");
  return new Date(iso).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    ...options,
  });
}

export function formatProjectRelativeTime(iso, t) {
  if (!iso) return tx(t, "projects.relative.noActivity", {}, "No recent activity");

  const date = new Date(iso);
  const diff = Date.now() - date.getTime();
  const absDiff = Math.abs(diff);
  const future = diff < 0;

  if (absDiff < 60_000) {
    return future
      ? tx(t, "projects.relative.inUnderMinute", {}, "in under a minute")
      : tx(t, "projects.relative.justNow", {}, "just now");
  }
  if (absDiff < 3_600_000) {
    const minutes = Math.floor(absDiff / 60_000);
    return future
      ? tx(t, "projects.relative.inMinutes", { count: minutes }, "in {count}m")
      : tx(t, "projects.relative.minutesAgo", { count: minutes }, "{count}m ago");
  }
  if (absDiff < 86_400_000) {
    const hours = Math.floor(absDiff / 3_600_000);
    return future
      ? tx(t, "projects.relative.inHours", { count: hours }, "in {count}h")
      : tx(t, "projects.relative.hoursAgo", { count: hours }, "{count}h ago");
  }

  const days = Math.floor(absDiff / 86_400_000);
  return future
    ? tx(t, "projects.relative.inDays", { count: days }, "in {count}d")
    : tx(t, "projects.relative.daysAgo", { count: days }, "{count}d ago");
}

export function formatCurrency(amount) {
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: amount >= 100 ? 0 : 2,
  }).format(Number(amount || 0));
}

export function healthTone(health) {
  if (health === "green") return "success";
  if (health === "yellow") return "warning";
  if (health === "red") return "danger";
  return "muted";
}

export function projectStateTone(state) {
  return state === "active" ? "success" : "muted";
}

export function threadTone(state) {
  if (state === "Running") return "signal";
  if (state === "Done" || state === "Completed") return "success";
  if (state === "Failed") return "danger";
  return "warning";
}

export function formatProjectHealth(health, t) {
  return formatEnumLabel(health, t, {
    labels: PROJECT_HEALTH_LABELS,
    keyPrefix: "projects.health",
  });
}

export function formatProjectState(state, t) {
  return formatEnumLabel(state, t, {
    labels: PROJECT_STATE_LABELS,
    keyPrefix: "projects.status",
  });
}

export function formatProjectRole(role, t) {
  return formatEnumLabel(role, t, {
    labels: PROJECT_ROLE_LABELS,
    keyPrefix: "projects.projectRole",
  });
}

export function formatThreadState(state, t) {
  return formatEnumLabel(state, t, {
    labels: THREAD_STATE_LABELS,
    keyPrefix: "projects.threadState",
  });
}

export function formatThreadType(type, t) {
  return formatEnumLabel(type, t, {
    labels: THREAD_TYPE_LABELS,
    keyPrefix: "projects.thread.type",
    defaultKey: "mission_run",
    translationKeys: { mission_run: "projects.thread.type.missionRun" },
    unknownLabel: (value) => String(value || "").replace(/_/g, " "),
  });
}

export function formatMessageRole(role, t) {
  return formatEnumLabel(role, t, {
    labels: MESSAGE_ROLE_LABELS,
    keyPrefix: "projects.role",
    defaultKey: "system",
  });
}

export function parseMissionRunGoal(goal) {
  const text = String(goal || "").trim();
  if (!text) return null;

  const markdownMatch = text.match(/^#\s*Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);
  if (markdownMatch) {
    return {
      missionName: markdownMatch[1].trim(),
      missionBrief: markdownMatch[2].trim(),
    };
  }

  const plainMatch = text.match(/^Mission:\s*(.+?)\s+Goal:\s*([\s\S]+)$/i);
  if (plainMatch) {
    return {
      missionName: plainMatch[1].trim(),
      missionBrief: plainMatch[2].trim(),
    };
  }

  return null;
}

export function threadPresentation(thread, t) {
  const parsedMission = parseMissionRunGoal(thread?.goal);

  if (parsedMission) {
    return {
      title: parsedMission.missionName,
      subtitle: tx(t, "projects.thread.missionRun", {}, "Mission run"),
      brief: parsedMission.missionBrief,
    };
  }

  return {
    title:
      thread?.title ||
      thread?.goal ||
      tx(t, "projects.thread.generatedTitle", { id: (thread?.id || "").slice(0, 8) }, "Thread {id}"),
    subtitle: thread?.thread_type
      ? formatThreadType(thread.thread_type, t)
      : tx(t, "projects.thread.generic", {}, "Thread"),
    brief: thread?.title && thread?.goal && thread.title !== thread.goal ? thread.goal : "",
  };
}

export function summarizeOverview(overview) {
  const counts = overview?.lifecycleCounts;

  return {
    totalProjects: counts?.total ?? 0,
    activeProjects: counts?.active ?? 0,
    archivedProjects: counts?.archived ?? 0,
  };
}

export function projectCount(t, key, count) {
  return tx(t, `projects.count.${key}`, { count: count || 0 }, "{count}");
}

export function messageContent(message) {
  if (!message) return "";
  if (typeof message.content === "string") return message.content;
  if (message.content == null) return "";
  try {
    return JSON.stringify(message.content, null, 2);
  } catch {
    return String(message.content);
  }
}

export function formatMetricValue(metric, t) {
  if (!metric) return tx(t, "projects.metric.notSet", {}, "Not set");

  const unit = metric.unit ? ` ${metric.unit}` : "";
  const current = metric.current != null ? `${metric.current}${unit}` : tx(t, "projects.metric.notSet", {}, "Not set");
  const target = metric.target != null ? `${metric.target}${unit}` : null;

  return target ? `${current} / ${target}` : current;
}
