function formatFallback(text, params = {}) {
  return text.replace(/\{(\w+)\}/g, (match, name) =>
    params[name] !== undefined ? params[name] : match
  );
}

function tx(t, key, params, fallback) {
  return typeof t === "function" ? t(key, params) : formatFallback(fallback, params);
}

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

export function missionTone(status) {
  if (status === "Active") return "signal";
  if (status === "Paused") return "warning";
  if (status === "Completed") return "success";
  if (status === "Failed") return "danger";
  return "muted";
}

export function threadTone(state) {
  if (state === "Running") return "signal";
  if (state === "Done" || state === "Completed") return "success";
  if (state === "Failed") return "danger";
  return "warning";
}

export function formatProjectHealth(health, t) {
  const key = String(health || "unknown").toLowerCase();
  const fallbackByKey = {
    green: "Healthy",
    yellow: "Needs review",
    red: "At risk",
    muted: "Archived",
    steady: "Steady",
    unknown: "Unknown",
  };
  const fallback = fallbackByKey[key];
  if (!fallback) return String(health || fallbackByKey.unknown);
  return tx(t, `projects.health.${key}`, {}, fallback);
}

export function formatMissionStatus(status, t) {
  const key = String(status || "unknown").toLowerCase();
  const fallbackByKey = {
    active: "Active",
    paused: "Paused",
    completed: "Completed",
    failed: "Failed",
    unknown: "Unknown",
  };
  const fallback = fallbackByKey[key];
  if (!fallback) return String(status || fallbackByKey.unknown);
  return tx(t, `projects.status.${key}`, {}, fallback);
}

export function formatThreadState(state, t) {
  const key = String(state || "unknown").toLowerCase();
  const fallbackByKey = {
    running: "Running",
    done: "Done",
    completed: "Completed",
    failed: "Failed",
    unknown: "Unknown",
  };
  const fallback = fallbackByKey[key];
  if (!fallback) return String(state || fallbackByKey.unknown);
  return tx(t, `projects.threadState.${key}`, {}, fallback);
}

export function formatThreadType(type, t) {
  const key = String(type || "mission_run").toLowerCase();
  if (key === "mission_run") {
    return tx(t, "projects.thread.type.missionRun", {}, "Mission run");
  }
  return String(type || "").replace(/_/g, " ");
}

export function formatMessageRole(role, t) {
  const key = String(role || "system").toLowerCase();
  const fallbackByKey = {
    system: "System",
    user: "User",
    assistant: "Assistant",
    tool: "Tool",
  };
  const fallback = fallbackByKey[key];
  if (!fallback) return String(role || fallbackByKey.system);
  return tx(t, `projects.role.${key}`, {}, fallback);
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
  const projects = overview?.projects || [];
  const totalSpend = projects.reduce((sum, project) => sum + Number(project.cost_today_usd || 0), 0);
  const activeMissions = projects.reduce((sum, project) => sum + Number(project.active_missions || 0), 0);
  const threadsToday = projects.reduce((sum, project) => sum + Number(project.threads_today || 0), 0);
  const pendingGates = projects.reduce((sum, project) => sum + Number(project.pending_gates || 0), 0);
  const failures24h = projects.reduce((sum, project) => sum + Number(project.failures_24h || 0), 0);

  return {
    totalProjects: projects.length,
    activeMissions,
    threadsToday,
    totalSpend,
    pendingGates,
    failures24h,
    attentionCount: overview?.attention?.length || 0,
  };
}

export function missionStatusCounts(missions = []) {
  return missions.reduce(
    (counts, mission) => {
      if (mission?.status === "Active") counts.active += 1;
      else if (mission?.status === "Paused") counts.paused += 1;
      else if (mission?.status === "Completed") counts.completed += 1;
      else if (mission?.status === "Failed") counts.failed += 1;
      return counts;
    },
    { active: 0, paused: 0, completed: 0, failed: 0 }
  );
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
