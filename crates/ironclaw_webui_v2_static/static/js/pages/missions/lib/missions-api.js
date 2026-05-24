// Mission endpoints depend on v1 `/api/engine/missions`. TODO stubs.

export function fetchProjectsOverview() {
  return Promise.resolve({ projects: [], todo: true });
}
export function fetchMissions({ projectId: _projectId } = {}) {
  return Promise.resolve({ missions: [], todo: true });
}
export function fetchMissionDetail(_missionId) {
  return Promise.resolve(null);
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
