// Workspace endpoints depend on v1 `/api/workspace/*`. TODO stubs.

export function listWorkspace(_path = "") {
  return Promise.resolve({ entries: [], todo: true });
}
export function readWorkspaceFile(_path) {
  return Promise.resolve({ content: "", todo: true });
}
export function writeWorkspaceFile(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 workspace endpoint" });
}
export function searchWorkspace(_query, _limit = 20) {
  return Promise.resolve({ matches: [], todo: true });
}
