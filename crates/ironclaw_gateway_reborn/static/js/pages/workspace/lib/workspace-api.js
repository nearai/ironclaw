import { apiFetch } from "../../../lib/api.js";

function buildUrl(path, params = {}) {
  const url = new URL(path, window.location.origin);
  Object.entries(params).forEach(([key, value]) => {
    if (value !== undefined && value !== null) {
      url.searchParams.set(key, value);
    }
  });
  return `${url.pathname}${url.search}`;
}

export function listWorkspace(path = "") {
  return apiFetch(buildUrl("/api/memory/list", { path }));
}

export function readWorkspaceFile(path) {
  return apiFetch(buildUrl("/api/memory/read", { path }));
}

export function writeWorkspaceFile({ path, content }) {
  return apiFetch("/api/memory/write", {
    method: "POST",
    body: JSON.stringify({ path, content }),
  });
}

export function searchWorkspace(query, limit = 20) {
  return apiFetch("/api/memory/search", {
    method: "POST",
    body: JSON.stringify({ query, limit }),
  });
}
