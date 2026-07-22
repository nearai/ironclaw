// Admin user-management client for the v2 `/api/webchat/v2/admin/*` routes
// (backed by `ironclaw_product_workflow::AdminUserService`). Authorization
// (operator token or admin/owner role) and last-admin protection are enforced
// server-side; a non-admin caller receives 403 here.
//
// The server DTO keys the user by `user_id`; the admin components read
// `user.id`, so every record is normalized to carry both.

import { apiFetch } from "../../../lib/api";

const ADMIN_BASE = "/api/webchat/v2/admin";
const EXTENSION_CONFIGURATION_BASE =
  "/api/webchat/v2/operator/extension-configuration";

function normalizeUser(record) {
  if (!record) return record;
  return { ...record, id: record.user_id };
}

// Lists one bounded page of users. The server clamps `limit` and returns a
// `next_cursor` for the following page; callers that omit params get the
// server's default page size. Backward compatible: `fetchAdminUsers()` with no
// args still requests the base route with no query string.
export async function fetchAdminUsers(params) {
  const query = new URLSearchParams();
  if (params?.status) query.set("status", params.status);
  if (params?.limit != null) query.set("limit", String(params.limit));
  if (params?.cursor) query.set("cursor", params.cursor);
  const suffix = query.toString() ? `?${query.toString()}` : "";
  const response = await apiFetch(`${ADMIN_BASE}/users${suffix}`);
  const users = Array.isArray(response?.users) ? response.users.map(normalizeUser) : [];
  return { users, total: users.length, nextCursor: response?.next_cursor ?? null };
}

export async function fetchAdminUser(id) {
  if (!id) return null;
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}`);
  return normalizeUser(response?.user);
}

export async function createAdminUser(payload) {
  const response = await apiFetch(`${ADMIN_BASE}/users`, {
    method: "POST",
    body: JSON.stringify({
      email: payload?.email,
      display_name: payload?.display_name,
      role: payload?.role || "member",
    }),
  });
  return normalizeUser(response?.user);
}

export async function createManagedAgent(payload) {
  const response = await apiFetch(`${ADMIN_BASE}/agents`, {
    method: "POST",
    body: JSON.stringify({ display_name: payload?.display_name }),
  });
  return normalizeUser(response?.user);
}

// Role changes route to the dedicated role endpoint; any other profile change
// (display_name / metadata) is a PATCH. The admin UI only ever sends `{ role }`
// today, but routing by payload keeps the client honest if that changes.
export async function updateAdminUser(id, payload) {
  if (payload && Object.prototype.hasOwnProperty.call(payload, "role")) {
    const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}/role`, {
      method: "POST",
      body: JSON.stringify({ role: payload.role }),
    });
    return normalizeUser(response?.user);
  }
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify({
      display_name: payload?.display_name,
      metadata: payload?.metadata,
    }),
  });
  return normalizeUser(response?.user);
}

export async function deleteAdminUser(id) {
  return apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export async function suspendAdminUser(id) {
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}/status`, {
    method: "POST",
    body: JSON.stringify({ status: "suspended" }),
  });
  return normalizeUser(response?.user);
}

export async function activateAdminUser(id) {
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}/status`, {
    method: "POST",
    body: JSON.stringify({ status: "active" }),
  });
  return normalizeUser(response?.user);
}

// --- Per-user secret provisioning -------------------------------------------

export async function fetchUserSecrets(userId) {
  const response = await apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets`);
  return Array.isArray(response?.secrets) ? response.secrets : [];
}

export async function putUserSecret(userId, handle, value) {
  const response = await apiFetch(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets/${encodeURIComponent(handle)}`,
    { method: "PUT", body: JSON.stringify({ value }) },
  );
  return response?.secret;
}

export async function deleteUserSecret(userId, handle) {
  return apiFetch(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets/${encodeURIComponent(handle)}`,
    { method: "DELETE" },
  );
}

// --- Manifest-driven deployment configuration -----------------------------

export async function fetchExtensionAdminConfiguration() {
  const response = await apiFetch(EXTENSION_CONFIGURATION_BASE);
  return Array.isArray(response?.groups) ? response.groups : [];
}

export async function replaceExtensionAdminConfiguration(
  groupId,
  values,
  expectedRevision,
  idempotencyKey,
) {
  return apiFetch(
    `${EXTENSION_CONFIGURATION_BASE}/${encodeURIComponent(groupId)}`,
    {
      method: "PUT",
      body: JSON.stringify({
        values,
        expected_revision: expectedRevision,
        idempotency_key: idempotencyKey,
      }),
    },
  );
}

// --- Usage / analytics (out of scope for this port) --------------------------
//
// The usage dashboard is intentionally NOT part of this admin port. These
// exports remain as inert empty stubs so the (now-unrouted) dashboard/usage
// components still import cleanly; the corresponding sub-routes are dropped in
// `app/routes.ts`, so they are never rendered.

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
