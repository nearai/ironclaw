// All jobs endpoints depend on v1 `/api/jobs/*`. TODO stubs until
// the v2 jobs contract lands; the page renders an empty list.
//
// The payloads route through `demoJson` so DEMO-mode staging builds can serve
// fixtures from these same paths; production keeps the empty fallbacks (the
// flag is a build-time constant).

import { demoJson } from "../../../demo/bridge";

export function fetchJobs() {
  return demoJson("/api/jobs", { jobs: [], pagination: null, todo: true });
}
export function fetchJobsSummary() {
  return demoJson("/api/jobs/summary", {
    total: 0,
    active: 0,
    completed: 0,
    failed: 0,
    todo: true,
  });
}
export function fetchJobDetail(jobId) {
  return demoJson(`/api/jobs/${encodeURIComponent(jobId)}`, null);
}
export function cancelJob(_jobId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 jobs endpoint" });
}
export function restartJob(_jobId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 jobs endpoint" });
}
export function fetchJobEvents(jobId) {
  return demoJson(`/api/jobs/${encodeURIComponent(jobId)}/events`, { events: [], todo: true });
}
export function sendJobPrompt(_jobId, _payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 jobs endpoint" });
}
export function fetchJobFiles(jobId, path = "") {
  return demoJson(
    `/api/jobs/${encodeURIComponent(jobId)}/files/list?path=${encodeURIComponent(path)}`,
    { entries: [], todo: true }
  );
}
export function readJobFile(jobId, path) {
  return demoJson(
    `/api/jobs/${encodeURIComponent(jobId)}/files/read?path=${encodeURIComponent(path)}`,
    { content: "", todo: true }
  );
}
