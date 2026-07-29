// @ts-nocheck
// Mission endpoints depend on v1 `/api/engine/missions`. TODO stubs.
//
// The payloads route through `demoJson` so DEMO-mode staging builds can serve
// fixtures from these same paths; production keeps the empty fallbacks (the
// flag is a build-time constant).

import { demoJson } from "../../../demo/bridge";

export function fetchProjectsOverview() {
  return demoJson("/api/engine/projects/overview", { projects: [], todo: true });
}
export function fetchMissions({ projectId } = {}) {
  const query = projectId ? `?project_id=${encodeURIComponent(projectId)}` : "";
  return demoJson(`/api/engine/missions${query}`, { missions: [], todo: true });
}
export function fetchMissionDetail(missionId) {
  return demoJson(`/api/engine/missions/${encodeURIComponent(missionId)}`, null);
}
export function fireMission(_missionId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 missions endpoint" });
}
export function pauseMission(_missionId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 missions endpoint" });
}
export function resumeMission(_missionId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 missions endpoint" });
}
