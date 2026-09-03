// Admin user-management client for the v2 `/api/webchat/v2/admin/*` routes
// (backed by `ironclaw_assistant::AdminUserService`). Authorization
// (operator token or admin/owner role) and last-admin protection are enforced
// server-side; a non-admin caller receives 403 here.
//
// The server DTO keys the user by `user_id`; the admin components read
// `user.id`, so every record is normalized to carry both.

import { apiFetch, type ApiRecord } from "../../../lib/api";

const ADMIN_BASE = "/api/webchat/v2/admin";
const EXTENSION_CONFIGURATION_BASE =
  "/api/webchat/v2/operator/extension-configuration";

type AdminUserWire = ApiRecord & {
  user_id: string;
  token?: string;
};

type AdminUser = AdminUserWire & { id: string };

type AdminUsersResponse = ApiRecord & {
  users?: AdminUserWire[];
  next_cursor?: string | null;
};

type AdminPageOptions = {
  status?: string;
  limit?: number;
  cursor?: string;
  signal?: AbortSignal;
};

type ThreadScrapeSummary = ApiRecord & { thread_id: string };
type ThreadScrapePage = ApiRecord & {
  threads?: ThreadScrapeSummary[];
  next_cursor?: string | null;
};
type ThreadScrapeArtifact = ApiRecord & {
  thread_id: string;
  messages: Array<ApiRecord & { message_id: string }>;
};
type RequestOptions = { signal?: AbortSignal };
type AdminSecret = ApiRecord & { handle: string };
type ConfigurationField = ApiRecord & {
  handle: string;
  label?: string;
  value?: string;
  description?: string;
  secret?: boolean;
  provided?: boolean;
  required?: boolean;
};
type ConfigurationConsumer = ApiRecord & {
  package_id: string;
  display_name?: string;
  installed?: boolean;
};
type ConfigurationGroup = ApiRecord & {
  group_id: string;
  revision: number;
  fields: ConfigurationField[];
  used_by: ConfigurationConsumer[];
};

function normalizeUser(record: AdminUserWire): AdminUser;
function normalizeUser(record: null | undefined): null | undefined;
function normalizeUser(record: AdminUserWire | null | undefined) {
  if (!record) return record;
  return { ...record, id: record.user_id };
}

// Lists one bounded page of users. The server clamps `limit` and returns a
// `next_cursor` for the following page; callers that omit params get the
// server's default page size. Backward compatible: `fetchAdminUsers()` with no
// args still requests the base route with no query string.
export async function fetchAdminUsers(
  params?: AdminPageOptions,
): Promise<{ users: AdminUser[]; total: number; nextCursor: string | null }> {
  const query = new URLSearchParams();
  if (params?.status) query.set("status", params.status);
  if (params?.limit != null) query.set("limit", String(params.limit));
  if (params?.cursor) query.set("cursor", params.cursor);
  const suffix = query.toString() ? `?${query.toString()}` : "";
  const response = await apiFetch<AdminUsersResponse>(
    `${ADMIN_BASE}/users${suffix}`,
    { signal: params?.signal },
  );
  const users = Array.isArray(response?.users) ? response.users.map(normalizeUser) : [];
  return { users, total: users.length, nextCursor: response?.next_cursor ?? null };
}

export async function fetchAdminUser(id: string): Promise<AdminUser | null> {
  if (!id) return null;
  const response = await apiFetch<{ user?: AdminUserWire }>(
    `${ADMIN_BASE}/users/${encodeURIComponent(id)}`,
  );
  return normalizeUser(response.user ?? null);
}

export async function fetchThreadScrapeThreads(
  userId: string,
  params?: AdminPageOptions,
): Promise<ThreadScrapePage> {
  const query = new URLSearchParams();
  if (params?.limit != null) query.set("limit", String(params.limit));
  if (params?.cursor) query.set("cursor", params.cursor);
  const suffix = query.toString() ? `?${query.toString()}` : "";
  return apiFetch<ThreadScrapePage>(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/thread-scrape/threads${suffix}`,
    { signal: params?.signal },
  );
}

export function fetchThreadScrapeArtifact(
  userId: string,
  threadId: string,
  params?: RequestOptions,
): Promise<ThreadScrapeArtifact> {
  return apiFetch<ThreadScrapeArtifact>(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/thread-scrape/threads/${encodeURIComponent(threadId)}/artifact`,
    { signal: params?.signal },
  );
}

export function fetchThreadScrapeRunArtifact(
  userId: string,
  threadId: string,
  runId: string,
  params?: RequestOptions,
): Promise<ApiRecord> {
  return apiFetch<ApiRecord>(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/thread-scrape/threads/${encodeURIComponent(threadId)}/runs/${encodeURIComponent(runId)}/artifact`,
    { signal: params?.signal },
  );
}

export async function createAdminUser(
  payload: ApiRecord,
): Promise<AdminUser & { token?: string }> {
  const response = await apiFetch<{ user: AdminUserWire; api_token?: string }>(
    `${ADMIN_BASE}/users`,
    {
      method: "POST",
      body: JSON.stringify({
        email: payload?.email,
        display_name: payload?.display_name,
        role: payload?.role || "member",
      }),
    },
  );
  // The one-time API bearer is exposed exactly once, here.
  return { ...normalizeUser(response?.user), token: response?.api_token };
}

// Role changes route to the dedicated role endpoint; any other profile change
// (display_name / metadata) is a PATCH. The admin UI only ever sends `{ role }`
// today, but routing by payload keeps the client honest if that changes.
export async function updateAdminUser(
  id: string,
  payload: ApiRecord,
): Promise<AdminUser> {
  if (payload && Object.prototype.hasOwnProperty.call(payload, "role")) {
    const response = await apiFetch<{ user: AdminUserWire }>(
      `${ADMIN_BASE}/users/${encodeURIComponent(id)}/role`,
      {
        method: "POST",
        body: JSON.stringify({ role: payload.role }),
      },
    );
    return normalizeUser(response?.user);
  }
  const response = await apiFetch<{ user: AdminUserWire }>(
    `${ADMIN_BASE}/users/${encodeURIComponent(id)}`,
    {
      method: "PATCH",
      body: JSON.stringify({
        display_name: payload?.display_name,
        metadata: payload?.metadata,
      }),
    },
  );
  return normalizeUser(response?.user);
}

export async function deleteAdminUser(id: string) {
  return apiFetch(`${ADMIN_BASE}/users/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export async function suspendAdminUser(id: string): Promise<AdminUser> {
  const response = await apiFetch<{ user: AdminUserWire }>(
    `${ADMIN_BASE}/users/${encodeURIComponent(id)}/status`,
    {
      method: "POST",
      body: JSON.stringify({ status: "suspended" }),
    },
  );
  return normalizeUser(response?.user);
}

export async function activateAdminUser(id: string): Promise<AdminUser> {
  const response = await apiFetch<{ user: AdminUserWire }>(
    `${ADMIN_BASE}/users/${encodeURIComponent(id)}/status`,
    {
      method: "POST",
      body: JSON.stringify({ status: "active" }),
    },
  );
  return normalizeUser(response?.user);
}

// This port issues the one-time API bearer only at user creation (a long-lived
// signed session bearer). Re-issuing a token for an existing user needs a
// dedicated endpoint that does not exist yet, so this rejects with a clear
// message rather than hitting a missing route. Tracked as a follow-up.
//
// The admin UI no longer calls this: the re-issue "Create Token" controls were
// removed from the existing-user views (user-detail + users-tab) so an admin
// can't trigger a guaranteed rejection. The export is kept only so the
// contract stays covered by admin-api.test.ts until a real endpoint lands.
export function createUserToken(_userId: string, _name: string) {
  return Promise.reject(
    new Error("API tokens are issued only when a user is created (re-issue not yet supported)"),
  );
}

// --- Per-user secret provisioning -------------------------------------------

export async function fetchUserSecrets(userId: string): Promise<AdminSecret[]> {
  const response = await apiFetch<{ secrets?: AdminSecret[] }>(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets`,
  );
  return Array.isArray(response?.secrets) ? response.secrets : [];
}

export async function putUserSecret(
  userId: string,
  handle: string,
  value: string,
): Promise<AdminSecret> {
  const response = await apiFetch<{ secret: AdminSecret }>(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets/${encodeURIComponent(handle)}`,
    { method: "PUT", body: JSON.stringify({ value }) },
  );
  return response?.secret;
}

export async function deleteUserSecret(userId: string, handle: string) {
  return apiFetch(
    `${ADMIN_BASE}/users/${encodeURIComponent(userId)}/secrets/${encodeURIComponent(handle)}`,
    { method: "DELETE" },
  );
}

// --- Manifest-driven deployment configuration -----------------------------

export async function fetchExtensionAdminConfiguration(): Promise<ConfigurationGroup[]> {
  const response = await apiFetch<{ groups?: ConfigurationGroup[] }>(
    EXTENSION_CONFIGURATION_BASE,
  );
  return Array.isArray(response?.groups) ? response.groups : [];
}

export async function replaceExtensionAdminConfiguration(
  groupId: string,
  values: Array<{ handle: string; value: string }>,
  expectedRevision: number,
  idempotencyKey: string,
): Promise<ConfigurationGroup> {
  return apiFetch<ConfigurationGroup>(
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
