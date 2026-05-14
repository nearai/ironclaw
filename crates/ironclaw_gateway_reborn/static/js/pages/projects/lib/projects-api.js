import { apiFetch } from "../../../lib/api.js";

function withPathParam(path, value) {
  return path.replace(":projectId", encodeURIComponent(value)).replace(":missionId", encodeURIComponent(value)).replace(":threadId", encodeURIComponent(value));
}

function buildUrl(path, params = {}) {
  const url = new URL(path, window.location.origin);
  Object.entries(params).forEach(([key, value]) => {
    if (value !== undefined && value !== null && value !== "") {
      url.searchParams.set(key, value);
    }
  });
  return `${url.pathname}${url.search}`;
}

export function fetchProjectsOverview() {
  return apiFetch("/api/engine/projects/overview");
}

export function fetchProjectDetail(projectId) {
  return apiFetch(withPathParam("/api/engine/projects/:projectId", projectId));
}

export function fetchProjectMissions(projectId) {
  return apiFetch(buildUrl("/api/engine/missions", { project_id: projectId }));
}

export function fetchProjectThreads(projectId) {
  return apiFetch(buildUrl("/api/engine/threads", { project_id: projectId }));
}

export function fetchProjectWidgets(projectId) {
  return apiFetch(withPathParam("/api/engine/projects/:projectId/widgets", projectId));
}

export function fetchMissionDetail(missionId) {
  return apiFetch(withPathParam("/api/engine/missions/:missionId", missionId));
}

export function fetchThreadDetail(threadId) {
  return apiFetch(withPathParam("/api/engine/threads/:threadId", threadId));
}

export function fireMission(missionId) {
  return apiFetch(withPathParam("/api/engine/missions/:missionId/fire", missionId), {
    method: "POST",
  });
}

export function pauseMission(missionId) {
  return apiFetch(withPathParam("/api/engine/missions/:missionId/pause", missionId), {
    method: "POST",
  });
}

export function resumeMission(missionId) {
  return apiFetch(withPathParam("/api/engine/missions/:missionId/resume", missionId), {
    method: "POST",
  });
}
