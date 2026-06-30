export function formatMissionDate(iso, options = {}) {
  if (!iso) return "Not scheduled";
  return new Date(iso).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    ...options,
  });
}

export function missionTone(status) {
  if (status === "Active") return "signal";
  if (status === "Paused") return "warning";
  if (status === "Completed") return "success";
  if (status === "Failed") return "danger";
  return "muted";
}

export function summarizeMissions(missions = []) {
  return missions.reduce(
    (summary, mission) => {
      summary.total += 1;
      if (mission.status === "Active") summary.active += 1;
      else if (mission.status === "Paused") summary.paused += 1;
      else if (mission.status === "Completed") summary.completed += 1;
      else if (mission.status === "Failed") summary.failed += 1;
      summary.threads += Number(mission.thread_count || mission.threads?.length || 0);
      return summary;
    },
    { total: 0, active: 0, paused: 0, completed: 0, failed: 0, threads: 0 }
  );
}

export function sortMissions(missions = []) {
  const statusRank = {
    Active: 0,
    Paused: 1,
    Failed: 2,
    Completed: 3,
  };

  return [...missions].sort((a, b) => {
    const rankDiff = (statusRank[a.status] ?? 4) - (statusRank[b.status] ?? 4);
    if (rankDiff !== 0) return rankDiff;
    return new Date(b.updated_at || 0).getTime() - new Date(a.updated_at || 0).getTime();
  });
}
