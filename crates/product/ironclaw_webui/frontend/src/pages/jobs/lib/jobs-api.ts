// All jobs endpoints depend on v1 `/api/jobs/*`. TODO stubs until
// the v2 jobs contract lands; the page renders an empty list.

export function fetchJobs() {
  return Promise.resolve({ jobs: [], pagination: null, todo: true });
}
export function fetchJobsSummary() {
  return Promise.resolve({ total: 0, active: 0, completed: 0, failed: 0, todo: true });
}
export function fetchJobDetail(_jobId) {
  return Promise.resolve(null);
}
export function cancelJob(_jobId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 jobs endpoint" });
}
export function restartJob(_jobId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 jobs endpoint" });
}
export function fetchJobEvents(_jobId) {
  return Promise.resolve({ events: [], todo: true });
}
export function sendJobPrompt(_jobId, _payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 jobs endpoint" });
}
export function fetchJobFiles(_jobId, _path = "") {
  return Promise.resolve({ entries: [], todo: true });
}
export function readJobFile(_jobId, _path) {
  return Promise.resolve({ content: "", todo: true });
}
