// Project endpoints depend on v1 `/api/engine/projects`. TODO stubs.

export function fetchProjectsOverview() {
  return Promise.resolve({ projects: [], todo: true });
}
export function fetchProjectDetail(_projectId) {
  return Promise.resolve(null);
}
export function fetchProjectMissions(_projectId) {
  return Promise.resolve({ missions: [], todo: true });
}
export function fetchProjectThreads(_projectId) {
  return Promise.resolve({ threads: [], todo: true });
}
export function fetchProjectWidgets(_projectId) {
  return Promise.resolve({ widgets: [], todo: true });
}
export function fetchMissionDetail(_missionId) {
  return Promise.resolve(null);
}
export function fetchThreadDetail(_threadId) {
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
