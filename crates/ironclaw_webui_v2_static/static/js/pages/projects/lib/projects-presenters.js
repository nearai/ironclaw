export function formatProjectDate(iso, options = {}) {
  if (!iso) return "Not available";
  return new Date(iso).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    ...options,
  });
}

export function formatProjectRelativeTime(iso) {
  if (!iso) return "No recent activity";

  const date = new Date(iso);
  const diff = Date.now() - date.getTime();
  const absDiff = Math.abs(diff);
  const future = diff < 0;

  if (absDiff < 60_000) return future ? "in under a minute" : "just now";
  if (absDiff < 3_600_000) {
    const minutes = Math.floor(absDiff / 60_000);
    return future ? `in ${minutes}m` : `${minutes}m ago`;
  }
  if (absDiff < 86_400_000) {
    const hours = Math.floor(absDiff / 3_600_000);
    return future ? `in ${hours}h` : `${hours}h ago`;
  }

  const days = Math.floor(absDiff / 86_400_000);
  return future ? `in ${days}d` : `${days}d ago`;
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

export function threadPresentation(thread) {
  const parsedMission = parseMissionRunGoal(thread?.goal);

  if (parsedMission) {
    return {
      title: parsedMission.missionName,
      subtitle: "Mission run",
      brief: parsedMission.missionBrief,
    };
  }

  return {
    title: thread?.title || thread?.goal || `Thread ${(thread?.id || "").slice(0, 8)}`,
    subtitle: thread?.thread_type ? String(thread.thread_type).replace(/_/g, " ") : "Thread",
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

export function compactCount(value, noun) {
  return `${value} ${noun}${value === 1 ? "" : "s"}`;
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

export function formatMetricValue(metric) {
  if (!metric) return "Not set";

  const unit = metric.unit ? ` ${metric.unit}` : "";
  const current = metric.current != null ? `${metric.current}${unit}` : "Not set";
  const target = metric.target != null ? `${metric.target}${unit}` : null;

  return target ? `${current} / ${target}` : current;
}
