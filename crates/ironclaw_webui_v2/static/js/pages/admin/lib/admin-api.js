// Admin user-management client for the v2 `/api/webchat/v2/admin/*` routes
// (backed by `ironclaw_product_workflow::AdminUserService`). Authorization
// (operator token or admin/owner role) and last-admin protection are enforced
// server-side; a non-admin caller receives 403 here.
//
// The server DTO keys the user by `user_id`; the admin components read
// `user.id`, so every record is normalized to carry both.

import { apiFetch } from "../../../lib/api.js";

const ADMIN_BASE = "/api/webchat/v2/admin";

function normalizeUser(record) {
  if (!record) return record;
  return { ...record, id: record.user_id };
}

export async function fetchAdminUsers() {
  const response = await apiFetch(`${ADMIN_BASE}/users`);
  const users = Array.isArray(response?.users) ? response.users.map(normalizeUser) : [];
  return { users, total: users.length };
}

export async function fetchAdminUser(id) {
  if (!id) return null;
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}`);
  return normalizeUser(response?.user);
}

export async function createAdminUser(payload) {
  const response = await apiFetch(`${ADMIN_BASE}/users`, {
    method: "POST",
    body: {
      email: payload?.email,
      display_name: payload?.display_name,
      role: payload?.role || "member",
    },
  });
  // The one-time API bearer is exposed exactly once, here.
  return { ...normalizeUser(response?.user), token: response?.api_token };
}

// Role changes route to the dedicated role endpoint; any other profile change
// (display_name / metadata) is a PATCH. The admin UI only ever sends `{ role }`
// today, but routing by payload keeps the client honest if that changes.
export async function updateAdminUser(id, payload) {
  if (payload && Object.prototype.hasOwnProperty.call(payload, "role")) {
    const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}/role`, {
      method: "POST",
      body: { role: payload.role },
    });
    return normalizeUser(response?.user);
  }
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: {
      display_name: payload?.display_name,
      metadata: payload?.metadata,
    },
  });
  return normalizeUser(response?.user);
}

export async function deleteAdminUser(id) {
  return apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export async function suspendAdminUser(id) {
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}/status`, {
    method: "POST",
    body: { status: "suspended" },
  });
  return normalizeUser(response?.user);
}

export async function activateAdminUser(id) {
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}/status`, {
    method: "POST",
    body: { status: "active" },
  });
  return normalizeUser(response?.user);
}

// This port issues the one-time API bearer only at user creation (a long-lived
// signed session bearer). Re-issuing a token for an existing user needs a
// dedicated endpoint that does not exist yet, so this rejects with a clear
// message rather than hitting a missing route. Tracked as a follow-up.
export function createUserToken(_userId, _name) {
  return Promise.reject(
    new Error("API tokens are issued only when a user is created (re-issue not yet supported)"),
  );
}

// --- Per-user secret provisioning -------------------------------------------

export async function fetchUserSecrets(userId) {
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets`);
  return Array.isArray(response?.secrets) ? response.secrets : [];
}

export async function putUserSecret(userId, handle, value) {
  const response = await apiFetch(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets/${encodeURIComponent(handle)}`,
    { method: "PUT", body: { value } },
  );
  return response?.secret;
}

export async function deleteUserSecret(userId, handle) {
  return apiFetch(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets/${encodeURIComponent(handle)}`,
    { method: "DELETE" },
  );
}

// --- Usage / analytics (out of scope for this port) --------------------------
//
// The usage dashboard is intentionally NOT part of this admin port. These
// exports remain as inert empty stubs so the (now-unrouted) dashboard/usage
// components still import cleanly; the corresponding sub-routes are dropped in
// `app/routes.js`, so they are never rendered.

export function fetchUsageSummary() {
  return Promise.resolve({
    total_users: 0,
    active_users: 0,
    suspended_users: 0,
    admin_users: 0,
    total_jobs: 0,
    llm_calls: 0,
    total_cost_usd: 0,
    active_jobs: 0,
    uptime_seconds: 0,
    recent_users: [],
    todo: true,
  });
}

export function fetchUsage(_period = "day", _userId) {
  return Promise.resolve({ entries: [], todo: true });
}
