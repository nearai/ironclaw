import { apiFetch } from "../../../lib/api.js";

function withPathParam(path, value) {
  return path.replace(":jobId", encodeURIComponent(value));
}

function buildJobUrl(path, params = {}) {
  const url = new URL(path, window.location.origin);
  Object.entries(params).forEach(([key, value]) => {
    if (value !== undefined && value !== null) {
      url.searchParams.set(key, value);
    }
  });
  return `${url.pathname}${url.search}`;
}

export function fetchJobs() {
  return apiFetch("/api/jobs");
}

export function fetchJobsSummary() {
  return apiFetch("/api/jobs/summary");
}

export function fetchJobDetail(jobId) {
  return apiFetch(withPathParam("/api/jobs/:jobId", jobId));
}

export function cancelJob(jobId) {
  return apiFetch(withPathParam("/api/jobs/:jobId/cancel", jobId), {
    method: "POST",
  });
}

export function restartJob(jobId) {
  return apiFetch(withPathParam("/api/jobs/:jobId/restart", jobId), {
    method: "POST",
  });
}

export function fetchJobEvents(jobId) {
  return apiFetch(withPathParam("/api/jobs/:jobId/events", jobId));
}

export function sendJobPrompt(jobId, { content, done = false }) {
  return apiFetch(withPathParam("/api/jobs/:jobId/prompt", jobId), {
    method: "POST",
    body: JSON.stringify({ content, done }),
  });
}

export function fetchJobFiles(jobId, path = "") {
  return apiFetch(buildJobUrl(withPathParam("/api/jobs/:jobId/files/list", jobId), { path }));
}

export function readJobFile(jobId, path) {
  return apiFetch(buildJobUrl(withPathParam("/api/jobs/:jobId/files/read", jobId), { path }));
}
