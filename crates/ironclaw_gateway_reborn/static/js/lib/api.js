const TOKEN_KEY = "ironclaw_token";

export class ApiError extends Error {
  constructor(message, { status, statusText, body, headers } = {}) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
    this.headers = headers;
  }
}

export function readStoredToken() {
  return sessionStorage.getItem(TOKEN_KEY) || "";
}

export function storeToken(token) {
  if (token) {
    sessionStorage.setItem(TOKEN_KEY, token);
  } else {
    sessionStorage.removeItem(TOKEN_KEY);
  }
}

export async function apiFetch(path, options = {}) {
  const token = readStoredToken();
  const headers = new Headers(options.headers || {});
  headers.set("Accept", "application/json");
  if (options.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  const response = await fetch(path, { credentials: "same-origin", ...options, headers });
  if (!response.ok) {
    const body = await response.text().catch(() => "");
    throw new ApiError(body || response.statusText, {
      status: response.status,
      statusText: response.statusText,
      body,
      headers: response.headers,
    });
  }
  const contentType = response.headers.get("content-type") || "";
  return contentType.includes("application/json") ? response.json() : response.text();
}

export function fetchAuthProviders() {
  return apiFetch("/auth/providers");
}

export function fetchProfile() {
  return apiFetch("/api/profile");
}

export async function logoutSession() {
  const response = await fetch("/auth/logout", {
    method: "POST",
    credentials: "include",
  });
  if (!response.ok) {
    const message = await response.text().catch(() => response.statusText);
    throw new Error(message || response.statusText);
  }
  return response.text();
}

// --- Gateway status ---

export function gatewayStatus() {
  return apiFetch("/api/gateway/status");
}

// --- Threads ---

export function fetchThreads() {
  return apiFetch("/api/chat/threads");
}

export function createThread() {
  return apiFetch("/api/chat/thread/new", { method: "POST" });
}

export function deleteThread(threadId) {
  return apiFetch(`/api/chat/thread/${encodeURIComponent(threadId)}`, {
    method: "DELETE",
  });
}

// --- History ---

export function fetchHistory({ threadId, limit = 50, before } = {}) {
  const url = new URL("/api/chat/timeline", window.location.origin);
  if (threadId) url.searchParams.set("thread_id", threadId);
  if (limit) url.searchParams.set("limit", String(limit));
  if (before) url.searchParams.set("before", before);
  return apiFetch(url.pathname + url.search);
}

// --- Send message ---

export function sendMessage({ content, threadId, timezone, images = [], attachments = [] }) {
  return apiFetch("/api/chat/turn", {
    method: "POST",
    body: JSON.stringify({
      content,
      thread_id: threadId || undefined,
      timezone: timezone || Intl.DateTimeFormat().resolvedOptions().timeZone,
      images,
      attachments,
    }),
  });
}

export function cancelRun({ threadId, runId, reason } = {}) {
  return apiFetch("/api/chat/run/cancel", {
    method: "POST",
    body: JSON.stringify({
      thread_id: threadId || undefined,
      run_id: runId || undefined,
      reason: reason || undefined,
    }),
  });
}

// --- Approval ---

export function sendApproval({ requestId, action, threadId }) {
  return apiFetch("/api/chat/approval", {
    method: "POST",
    body: JSON.stringify({ request_id: requestId, action, thread_id: threadId || undefined }),
  });
}

// --- Gate resolve ---

export function resolveGate({ requestId, resolution, threadId }) {
  const payload = {
    request_id: requestId,
    thread_id: threadId || undefined,
    ...gateResolutionPayload(resolution),
  };

  return apiFetch("/api/chat/gate/resolve", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

function gateResolutionPayload(resolution) {
  if (resolution === "approve" || resolution === "approved") {
    return { resolution: "approved", always: false };
  }
  if (resolution === "always") {
    return { resolution: "approved", always: true };
  }
  if (resolution === "deny" || resolution === "denied") {
    return { resolution: "denied" };
  }
  return { resolution };
}
