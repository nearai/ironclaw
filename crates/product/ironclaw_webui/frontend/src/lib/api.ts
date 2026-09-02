// WebChat v2 ingress client.
//
// Every function in this module targets a `/api/webchat/v2/*` route
// defined by issue #3815, a v2-owned `/auth/*` route mounted by
// `ironclaw_webui::webui_v2_auth_router`, or a
// Reborn product-auth route mounted by the host WebUI gateway. The module
// deliberately contains no `/api/chat`, `/api/engine`, or
// `/api/profile` paths — the hard non-goal of issue #3886 still
// stands for v1 gateway routes that lack a v2 counterpart.
//
// Request/response shapes mirror the Rust DTOs in
// `ironclaw_assistant::product_surface_inbound` and
// `ironclaw_assistant::reborn_services::types`. The error
// envelope mirrors `ProductSurfaceError`.

const TOKEN_KEY = "ironclaw_token";
const V2_BASE = "/api/webchat/v2";

function encodePathSegment(value: unknown): string {
  return encodeURIComponent(String(value));
}

type ApiErrorPayload = {
  error?: unknown;
  field?: unknown;
  kind?: unknown;
  validation_code?: unknown;
};

export interface ApiErrorOptions {
  status?: number;
  statusText?: string;
  body?: string;
  headers?: Headers;
  payload?: unknown;
}

export class ApiError extends Error {
  status?: number;
  statusText?: string;
  body?: string;
  headers?: Headers;
  payload?: unknown;

  constructor(
    message: string,
    { status, statusText, body, headers, payload }: ApiErrorOptions = {},
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
    this.headers = headers;
    // Parsed ProductSurfaceError when the server returned JSON in
    // the documented shape. Undefined for non-JSON 5xx / proxy errors.
    this.payload = payload;
  }
}

export function readStoredToken() {
  return sessionStorage.getItem(TOKEN_KEY) || "";
}

export function storeToken(token: string) {
  if (token) {
    sessionStorage.setItem(TOKEN_KEY, token);
  } else {
    sessionStorage.removeItem(TOKEN_KEY);
  }
}

// Generate a client action id (idempotency key) for mutating requests.
// Must be a non-empty token with no control characters; `crypto.randomUUID`
// satisfies the validator in `product_surface_inbound::parse_client_action_id`.
export function clientActionId() {
  if (typeof crypto !== "undefined" && crypto && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  const bytes = new Uint8Array(16);
  if (typeof crypto !== "undefined" && crypto && typeof crypto.getRandomValues === "function") {
    // Must be invoked on `crypto` — an unbound call throws `Illegal invocation`.
    crypto.getRandomValues(bytes);
  } else {
    // No Web Crypto at all. Math.random suffices: the id only needs to be
    // unique per action, never constant (a constant id dedupes distinct sends).
    for (let i = 0; i < bytes.length; i += 1) {
      bytes[i] = Math.floor(Math.random() * 256);
    }
  }
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

async function parseErrorBody(response: Response) {
  const text = await response.text().catch(() => "");
  if (!text) return { text: "", payload: undefined };
  const contentType = response.headers.get("content-type") || "";
  if (!contentType.includes("application/json")) {
    return { text, payload: undefined };
  }
  try {
    return { text, payload: JSON.parse(text) };
  } catch (_) {
    return { text, payload: undefined };
  }
}

// Turn a snake_case / kebab-case wire token into a readable phrase, e.g.
// `service_unavailable` -> "Service unavailable".
function humanizeErrorToken(token: unknown) {
  return String(token)
    .replace(/[_-]+/g, " ")
    .trim()
    .replace(/^\w/, (char) => char.toUpperCase());
}

// Derive a human-readable message from a WebChat v2 error response.
//
// The wire envelope (`ironclaw_webui_v2::WebUiV2HttpErrorBody`) carries only
// snake_case enum codes — `kind` (the user-renderable family, e.g.
// `service_unavailable`), `error` (a coarse code), and an optional
// `validation_code` + `field` — never prose. Throwing the raw JSON body as the
// error message means a dialog shows `{"error":"...","kind":"..."}`, which reads
// as "no error" to a user. Humanize the most specific token instead, and only
// fall back to a non-JSON body when it is short enough to be a real message.
export function describeApiError({
  payload,
  body,
  statusText,
}: Pick<ApiErrorOptions, "payload" | "body" | "statusText"> = {}) {
  if (payload && typeof payload === "object") {
    const errorPayload = payload as ApiErrorPayload;
    if (errorPayload.validation_code) {
      const base = humanizeErrorToken(errorPayload.validation_code);
      return errorPayload.field ? `${base} (${String(errorPayload.field)})` : base;
    }
    const code = errorPayload.kind || errorPayload.error;
    if (code) {
      const base = humanizeErrorToken(code);
      return errorPayload.field ? `${base} (${String(errorPayload.field)})` : base;
    }
  }
  const trimmed = (body || "").trim();
  if (
    trimmed &&
    trimmed.length <= 200 &&
    !trimmed.startsWith("{") &&
    !trimmed.startsWith("[")
  ) {
    return trimmed;
  }
  return statusText || "Request failed";
}

export async function apiFetch(path: string | URL, options: RequestInit = {}) {
  const token = readStoredToken();
  const headers = new Headers(options.headers || {});
  headers.set("Accept", "application/json");
  if (options.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  const response = await fetch(path, {
    credentials: "same-origin",
    ...options,
    headers,
  });

  if (!response.ok) {
    const { text, payload } = await parseErrorBody(response);
    throw new ApiError(
      describeApiError({ payload, body: text, statusText: response.statusText }),
      {
        status: response.status,
        statusText: response.statusText,
        body: text,
        headers: response.headers,
        payload,
      },
    );
  }

  const contentType = response.headers.get("content-type") || "";
  return contentType.includes("application/json")
    ? response.json()
    : response.text();
}

// --- Threads ---

// The deployment's authenticated-session channel, learned from
// `GET /session` (`session_channel_extension_id`). The send path plugs it
// into the generic session-inbound route; the frontend never hardcodes a
// channel name. Absent until the session loads — sends fail closed with a
// clear error rather than guessing a channel.
let sessionChannelExtensionId = "";

export function setSessionChannelExtensionId(extensionId: string | null | undefined) {
  sessionChannelExtensionId = extensionId || "";
}

/** The deployment's authenticated-session channel id (empty until the
 * session loads). Generic UI keys per-channel affordances — the session
 * message route, this browser's notification enrollment — off this value
 * instead of any hardcoded channel name. */
export function getSessionChannelExtensionId() {
  return sessionChannelExtensionId;
}

export async function fetchSession() {
  const session = await apiFetch(`${V2_BASE}/session`);
  setSessionChannelExtensionId(session?.session_channel_extension_id);
  return session;
}

export interface CreateThreadOptions {
  clientActionId?: string;
  requestedThreadId?: string;
  projectId?: string;
}

export function createThread({
  clientActionId: clientId,
  requestedThreadId,
  projectId,
}: CreateThreadOptions = {}) {
  const body: {
    client_action_id: string;
    requested_thread_id?: string;
    project_id?: string;
  } = { client_action_id: clientId || clientActionId() };
  if (requestedThreadId) body.requested_thread_id = requestedThreadId;
  // The backend authorizes the caller's access to this project before scoping
  // the new thread to it; the body only proposes it.
  if (projectId) body.project_id = projectId;
  return apiFetch(`${V2_BASE}/threads`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export interface ListThreadsOptions {
  limit?: number;
  cursor?: string;
  projectId?: string;
  candidateThreadId?: string;
  signal?: AbortSignal;
}

export function listThreads({
  limit,
  cursor,
  projectId,
  candidateThreadId,
  signal,
}: ListThreadsOptions = {}) {
  const url = new URL(`${V2_BASE}/threads`, window.location.origin);
  if (limit != null) url.searchParams.set("limit", String(limit));
  if (cursor) url.searchParams.set("cursor", cursor);
  if (projectId) url.searchParams.set("project_id", projectId);
  if (candidateThreadId) url.searchParams.set("candidate_thread_id", candidateThreadId);
  return apiFetch(url.pathname + url.search, { signal });
}

type ThreadOptions = { threadId?: string };
type AutomationOptions = { automationId?: string };
type ProjectOptions = { projectId?: string };

export function deleteThread({ threadId }: ThreadOptions = {}) {
  if (!threadId) {
    return Promise.reject(new Error("threadId is required"));
  }
  return apiFetch(`${V2_BASE}/threads/${encodeURIComponent(threadId)}`, {
    method: "DELETE",
  });
}

// --- Notification inbox ---

interface PaginatedRequestOptions {
  limit?: number;
  cursor?: string;
  signal?: AbortSignal;
}

export function listNotifications({
  limit,
  cursor,
  signal,
}: PaginatedRequestOptions = {}) {
  const url = new URL(`${V2_BASE}/notifications`, window.location.origin);
  if (limit != null) url.searchParams.set("limit", String(limit));
  if (cursor) url.searchParams.set("cursor", cursor);
  return apiFetch(url.pathname + url.search, { signal });
}

export function markNotificationRead(notificationId: string) {
  if (!notificationId) return Promise.reject(new Error("notificationId is required"));
  return apiFetch(
    `${V2_BASE}/notifications/${encodeURIComponent(notificationId)}/read`,
    { method: "POST" },
  );
}

export function markAllNotificationsRead() {
  return apiFetch(`${V2_BASE}/notifications/read-all`, { method: "POST" });
}

export function archiveNotification(notificationId: string) {
  if (!notificationId) return Promise.reject(new Error("notificationId is required"));
  return apiFetch(
    `${V2_BASE}/notifications/${encodeURIComponent(notificationId)}/archive`,
    { method: "POST" },
  );
}

// --- Project filesystem (download / navigation) ---

function projectFilesBase(threadId: string) {
  return `${V2_BASE}/threads/${encodeURIComponent(threadId)}/files`;
}

// List a directory under the thread's project workspace. `path` defaults to the
// workspace root server-side when omitted.
interface ProjectFileOptions extends ThreadOptions {
  path?: string;
}

export function listProjectFiles({ threadId, path }: ProjectFileOptions = {}) {
  if (!threadId) return Promise.reject(new Error("threadId is required"));
  const url = new URL(projectFilesBase(threadId), window.location.origin);
  if (path) url.searchParams.set("path", path);
  return apiFetch(url.pathname + url.search);
}

// Metadata for a single project path (used to show a chip's size/icon).
export function statProjectFile({ threadId, path }: ProjectFileOptions = {}) {
  if (!threadId || !path) {
    return Promise.reject(new Error("threadId and path are required"));
  }
  const url = new URL(`${projectFilesBase(threadId)}/stat`, window.location.origin);
  url.searchParams.set("path", path);
  return apiFetch(url.pathname + url.search);
}

// Same-origin relative URL for a project file's bytes. Feeds the shared
// `fetchAttachmentBlob` (which attaches the bearer) so the project workspace
// browser can reuse the attachment preview modal.
export function projectFileContentUrl({ threadId, path }: ProjectFileOptions = {}) {
  if (!threadId || !path) {
    throw new Error("projectFileContentUrl requires threadId and path");
  }
  const url = new URL(`${projectFilesBase(threadId)}/content`, window.location.origin);
  url.searchParams.set("path", path);
  return url.pathname + url.search;
}

// --- Automations ---

interface ListAutomationsOptions {
  limit?: number;
  runLimit?: number;
  includeCompleted?: boolean;
}

export function listAutomations({
  limit,
  runLimit,
  includeCompleted,
}: ListAutomationsOptions = {}) {
  const params = new URLSearchParams();
  if (limit != null) params.set("limit", String(limit));
  if (runLimit != null) params.set("run_limit", String(runLimit));
  if (includeCompleted === true) params.set("include_completed", "true");
  const query = params.toString();
  return apiFetch(`${V2_BASE}/automations${query ? `?${query}` : ""}`);
}

export function pauseAutomation({ automationId }: AutomationOptions = {}) {
  if (!automationId) {
    return Promise.reject(new Error("automationId is required"));
  }
  return apiFetch(`${V2_BASE}/automations/${encodeURIComponent(automationId)}/pause`, {
    method: "POST",
  });
}

export function runAutomation({ automationId }: AutomationOptions = {}) {
  if (!automationId) {
    return Promise.reject(new Error("automationId is required"));
  }
  return apiFetch(`${V2_BASE}/automations/${encodeURIComponent(automationId)}/run`, {
    method: "POST",
  });
}

export function resumeAutomation({ automationId }: AutomationOptions = {}) {
  if (!automationId) {
    return Promise.reject(new Error("automationId is required"));
  }
  return apiFetch(`${V2_BASE}/automations/${encodeURIComponent(automationId)}/resume`, {
    method: "POST",
  });
}

export function renameAutomation({
  automationId,
  name,
}: AutomationOptions & { name?: string } = {}) {
  if (!automationId) {
    return Promise.reject(new Error("automationId is required"));
  }
  if (!name) {
    return Promise.reject(new Error("name is required"));
  }
  return apiFetch(`${V2_BASE}/automations/${encodeURIComponent(automationId)}`, {
    method: "POST",
    body: JSON.stringify({ name }),
  });
}

export function deleteAutomation({ automationId }: AutomationOptions = {}) {
  if (!automationId) {
    return Promise.reject(new Error("automationId is required"));
  }
  return apiFetch(`${V2_BASE}/automations/${encodeURIComponent(automationId)}`, {
    method: "DELETE",
  });
}

// --- Projects (first-class entity + membership ACL) ---

const PROJECTS_BASE = `${V2_BASE}/projects`;

function projectPath(projectId: string) {
  return `${PROJECTS_BASE}/${encodeURIComponent(projectId)}`;
}

export function listProjects({ limit }: { limit?: number } = {}) {
  const url = new URL(PROJECTS_BASE, window.location.origin);
  if (limit != null) url.searchParams.set("limit", String(limit));
  return apiFetch(url.pathname + url.search);
}

export interface ProjectMutationOptions extends ProjectOptions {
  name?: string;
  description?: string | null;
  icon?: string | null;
  color?: string | null;
  metadata?: Record<string, unknown> | null;
  state?: string | null;
}

export function createProject({
  name,
  description,
  icon,
  color,
  metadata,
}: ProjectMutationOptions = {}) {
  if (!name) return Promise.reject(new Error("name is required"));
  const body: Omit<ProjectMutationOptions, "projectId" | "state"> = { name };
  if (description != null) body.description = description;
  if (icon != null) body.icon = icon;
  if (color != null) body.color = color;
  if (metadata != null) body.metadata = metadata;
  return apiFetch(PROJECTS_BASE, { method: "POST", body: JSON.stringify(body) });
}

export function getProject({ projectId }: ProjectOptions = {}) {
  if (!projectId) return Promise.reject(new Error("projectId is required"));
  return apiFetch(projectPath(projectId));
}

export function updateProject({
  projectId,
  name,
  description,
  icon,
  color,
  metadata,
  state,
}: ProjectMutationOptions = {}) {
  if (!projectId) return Promise.reject(new Error("projectId is required"));
  const body: Omit<ProjectMutationOptions, "projectId"> = {};
  if (name != null) body.name = name;
  if (description != null) body.description = description;
  if (icon != null) body.icon = icon;
  if (color != null) body.color = color;
  if (metadata != null) body.metadata = metadata;
  if (state != null) body.state = state;
  return apiFetch(projectPath(projectId), { method: "POST", body: JSON.stringify(body) });
}

export function deleteProject({ projectId }: ProjectOptions = {}) {
  if (!projectId) return Promise.reject(new Error("projectId is required"));
  return apiFetch(projectPath(projectId), { method: "DELETE" });
}

export function listProjectMembers({ projectId }: ProjectOptions = {}) {
  if (!projectId) return Promise.reject(new Error("projectId is required"));
  return apiFetch(`${projectPath(projectId)}/members`);
}

interface ProjectMemberOptions extends ProjectOptions {
  userId?: string;
  role?: string;
}

export function addProjectMember({
  projectId,
  userId,
  role,
}: ProjectMemberOptions = {}) {
  if (!projectId || !userId) {
    return Promise.reject(new Error("projectId and userId are required"));
  }
  if (!role) return Promise.reject(new Error("role is required"));
  return apiFetch(`${projectPath(projectId)}/members`, {
    method: "POST",
    body: JSON.stringify({ user_id: userId, role }),
  });
}

export function updateProjectMemberRole({
  projectId,
  userId,
  role,
}: ProjectMemberOptions = {}) {
  if (!projectId || !userId) {
    return Promise.reject(new Error("projectId and userId are required"));
  }
  if (!role) return Promise.reject(new Error("role is required"));
  return apiFetch(`${projectPath(projectId)}/members/${encodeURIComponent(userId)}`, {
    method: "POST",
    body: JSON.stringify({ role }),
  });
}

export function removeProjectMember({ projectId, userId }: ProjectMemberOptions = {}) {
  if (!projectId || !userId) {
    return Promise.reject(new Error("projectId and userId are required"));
  }
  return apiFetch(`${projectPath(projectId)}/members/${encodeURIComponent(userId)}`, {
    method: "DELETE",
  });
}

// --- Outbound delivery targets + notification channels ---

export function listOutboundDeliveryTargets() {
  return apiFetch(`${V2_BASE}/outbound/targets`);
}

export function getNotificationChannels() {
  return apiFetch(`${V2_BASE}/outbound/notification-channels`);
}

// `target_ids` is the sole canonical wire name for this full-replace body —
// no aliases (see CLAUDE.md's wire-contract naming rule).
//
// No `targetIds ?? []` default. `RebornSetNotificationChannelsRequest`
// deliberately omits `#[serde(default)]` on `target_ids` so an omitted field
// is a 400, never an implicit clear-all — and a client-side default would
// defeat exactly that guard, because the backend accepts `[]` as a valid,
// *intentional* full replace. A caller that forgot the argument would wipe
// every stored notification channel. Fail fast instead, the way the other
// mutations here reject a missing required argument.
export function setNotificationChannels({
  targetIds,
}: { targetIds?: unknown } = {}) {
  if (!Array.isArray(targetIds)) {
    return Promise.reject(
      new TypeError("targetIds must be an array of target ids (pass [] to clear)"),
    );
  }
  return apiFetch(`${V2_BASE}/outbound/notification-channels`, {
    method: "POST",
    body: JSON.stringify({ target_ids: targetIds }),
  });
}

// --- Notification setup (generic per-channel enrollment) ---
//
// One status/enable/disable surface for every channel, keyed by extension id.
// The `payload` bodies and the status `detail` are channel-opaque documents
// only the channel's own client interprets — nothing here names a channel.

interface NotificationSetupOptions {
  extensionId?: string;
  payload?: unknown;
}

export function getNotificationSetupStatus({
  extensionId,
}: NotificationSetupOptions = {}) {
  if (!extensionId) {
    return Promise.reject(new Error("extensionId is required"));
  }
  return apiFetch(
    `${V2_BASE}/channels/${encodeURIComponent(extensionId)}/notifications`,
  );
}

export function enableNotificationSetup({
  extensionId,
  payload,
}: NotificationSetupOptions = {}) {
  if (!extensionId) {
    return Promise.reject(new Error("extensionId is required"));
  }
  return apiFetch(
    `${V2_BASE}/channels/${encodeURIComponent(extensionId)}/notifications/enable`,
    {
      method: "POST",
      body: JSON.stringify({ payload }),
    },
  );
}

export function disableNotificationSetup({
  extensionId,
  payload,
}: NotificationSetupOptions = {}) {
  if (!extensionId) {
    return Promise.reject(new Error("extensionId is required"));
  }
  return apiFetch(
    `${V2_BASE}/channels/${encodeURIComponent(extensionId)}/notifications/disable`,
    {
      method: "POST",
      body: JSON.stringify({ payload }),
    },
  );
}

// --- Operator logs ---

export interface LogQueryOptions extends PaginatedRequestOptions {
  level?: string;
  target?: string;
  threadId?: string;
  runId?: string;
  turnId?: string;
  toolCallId?: string;
  toolName?: string;
  source?: string;
  tail?: boolean;
  follow?: boolean;
}

export function queryLogs({
  limit,
  cursor,
  level,
  target,
  threadId,
  runId,
  turnId,
  toolCallId,
  toolName,
  source,
  tail,
  follow,
  signal,
}: LogQueryOptions = {}) {
  const url = new URL(`${V2_BASE}/logs`, window.location.origin);
  if (limit != null) url.searchParams.set("limit", String(limit));
  if (cursor) url.searchParams.set("cursor", cursor);
  if (level) url.searchParams.set("level", level);
  if (target) url.searchParams.set("target", target);
  if (threadId) url.searchParams.set("thread_id", threadId);
  if (runId) url.searchParams.set("run_id", runId);
  if (turnId) url.searchParams.set("turn_id", turnId);
  if (toolCallId) url.searchParams.set("tool_call_id", toolCallId);
  if (toolName) url.searchParams.set("tool_name", toolName);
  if (source) url.searchParams.set("source", source);
  if (tail) url.searchParams.set("tail", "true");
  if (follow) url.searchParams.set("follow", "true");
  return apiFetch(url.pathname + url.search, { signal });
}

export function queryOperatorLogs({
  limit,
  cursor,
  level,
  target,
  threadId,
  runId,
  turnId,
  toolCallId,
  toolName,
  source,
  tail,
  follow,
  signal,
}: LogQueryOptions = {}) {
  const url = new URL(`${V2_BASE}/operator/logs`, window.location.origin);
  if (limit != null) url.searchParams.set("limit", String(limit));
  if (cursor) url.searchParams.set("cursor", cursor);
  if (level) url.searchParams.set("level", level);
  if (target) url.searchParams.set("target", target);
  if (threadId) url.searchParams.set("thread_id", threadId);
  if (runId) url.searchParams.set("run_id", runId);
  if (turnId) url.searchParams.set("turn_id", turnId);
  if (toolCallId) url.searchParams.set("tool_call_id", toolCallId);
  if (toolName) url.searchParams.set("tool_name", toolName);
  if (source) url.searchParams.set("source", source);
  if (tail) url.searchParams.set("tail", "true");
  if (follow) url.searchParams.set("follow", "true");
  return apiFetch(url.pathname + url.search, { signal });
}

// --- Messages ---

// `attachments` is an array of `ProductInboundAttachment`
// (`{ mime_type, filename, data_base64 }`). Omitted from the body when
// empty so a text-only send keeps the original wire shape.
export interface ProductInboundAttachment {
  mime_type: string;
  filename?: string | null;
  data_base64: string;
}

export interface SendMessageOptions {
  threadId: string;
  content: string;
  attachments?: ProductInboundAttachment[];
  clientActionId?: string;
}

export function sendMessage({
  threadId,
  content,
  attachments = [],
  clientActionId: clientId,
}: SendMessageOptions) {
  if (!sessionChannelExtensionId) {
    return Promise.reject(
      new Error("no session channel is configured for this deployment"),
    );
  }
  const body: {
    client_action_id: string;
    thread_id: string;
    content: string;
    attachments?: ProductInboundAttachment[];
  } = {
    client_action_id: clientId || clientActionId(),
    thread_id: threadId,
    content,
  };
  if (attachments.length > 0) {
    body.attachments = attachments;
  }
  return apiFetch(
    `${V2_BASE}/channels/${encodeURIComponent(sessionChannelExtensionId)}/messages`,
    {
      method: "POST",
      body: JSON.stringify(body),
    },
  );
}

// --- Product commands ---

export function listChatCommands() {
  return apiFetch(`${V2_BASE}/commands`);
}

export function executeChatCommand({
  threadId,
  text,
}: { threadId?: string | null; text: string }) {
  return apiFetch(
    `${V2_BASE}/threads/${encodeURIComponent(threadId ?? "")}/commands`,
    {
      method: "POST",
      body: JSON.stringify({ text }),
    },
  );
}

// --- Timeline ---

export function fetchTimeline({
  threadId,
  limit,
  cursor,
}: ThreadOptions & Omit<PaginatedRequestOptions, "signal"> = {}) {
  const url = new URL(
    `${V2_BASE}/threads/${encodePathSegment(threadId)}/timeline`,
    window.location.origin,
  );
  if (limit != null) url.searchParams.set("limit", String(limit));
  if (cursor) url.searchParams.set("cursor", cursor);
  return apiFetch(url.pathname + url.search);
}

interface RunOptions extends ThreadOptions {
  runId?: string;
}

export function fetchRunArtifact({ threadId, runId }: RunOptions = {}) {
  if (!threadId || !runId) {
    return Promise.reject(new Error("threadId and runId are required"));
  }
  return apiFetch(
    `${V2_BASE}/threads/${encodeURIComponent(threadId)}/runs/${encodeURIComponent(runId)}/artifact`,
  );
}

export function fetchThreadArtifact({ threadId }: ThreadOptions = {}) {
  if (!threadId) {
    return Promise.reject(new Error("threadId is required"));
  }
  return apiFetch(`${V2_BASE}/threads/${encodeURIComponent(threadId)}/artifact`);
}

// --- Attachments ---

// Path for one landed attachment's bytes. The (thread, message, attachment)
// triple addresses it: an attachment id is only unique within its message.
// Fails fast on a missing part rather than building a path with the literal
// "undefined" — this URL feeds `fetchAttachmentBlob`, which attaches the bearer,
// so an unintended path must never be requested.
interface AttachmentOptions extends ThreadOptions {
  messageId?: string;
  attachmentId?: string;
}

export function attachmentUrl({
  threadId,
  messageId,
  attachmentId,
}: AttachmentOptions = {}) {
  if (!threadId || !messageId || !attachmentId) {
    throw new Error("attachmentUrl requires threadId, messageId, and attachmentId");
  }
  return (
    `${V2_BASE}/threads/${encodeURIComponent(threadId)}` +
    `/messages/${encodeURIComponent(messageId)}` +
    `/attachments/${encodeURIComponent(attachmentId)}`
  );
}

// Fetch an attachment's bytes with the session bearer and return them as a
// `Blob`. `<img>`/`<audio>`/`<iframe>` cannot send an Authorization header, so
// (unlike SSE, which uses a `?token=` shim) the bytes are fetched here and the
// caller picks the CSP-appropriate representation (data URL for images/media,
// blob URL for PDF frames, text for text). Throws on a non-OK response so the
// caller can fall back to a placeholder.
export async function fetchAttachmentBlob(path: string | URL) {
  // The bearer is a critical sink: never attach it to an off-origin URL. The
  // caller always passes a relative same-origin path (`attachmentUrl(...)`);
  // reject anything that resolves cross-origin before sending the token.
  const url = new URL(path, window.location.origin);
  if (url.origin !== window.location.origin) {
    throw new ApiError("Invalid attachment URL.", {
      status: 400,
      statusText: "Bad Request",
    });
  }
  const token = readStoredToken();
  const headers = new Headers();
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  const response = await fetch(url.pathname + url.search, {
    credentials: "same-origin",
    headers,
  });
  if (!response.ok) {
    const { text, payload } = await parseErrorBody(response);
    throw new ApiError(
      describeApiError({ payload, body: text, statusText: response.statusText }),
      { status: response.status, statusText: response.statusText, body: text, payload },
    );
  }
  return await response.blob();
}

// Read a `Blob` into a `data:` URL. Used for images and media, whose CSP
// directives (`img-src`/`media-src 'self' data:`) allow data URLs but not
// `blob:` — and a data URL needs no `revokeObjectURL` lifecycle.
export function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") {
        resolve(reader.result);
      } else {
        reject(new Error("attachment data URL was not a string"));
      }
    };
    reader.onerror = () => reject(reader.error || new Error("attachment read failed"));
    reader.readAsDataURL(blob);
  });
}

// Convenience: fetch an attachment's bytes and return a `data:` URL for an
// `<img>` thumbnail. CSP-safe (`img-src 'self' data:`); never a `blob:` URL.
export async function fetchAttachmentDataUrl(path: string | URL) {
  return blobToDataUrl(await fetchAttachmentBlob(path));
}

// --- Streaming (SSE) ---

export function eventStreamRequest({
  threadId,
  connectionId,
}: ThreadOptions & { connectionId?: string } = {}) {
  const url = new URL(
    `${V2_BASE}/threads/${encodePathSegment(threadId)}/events`,
    window.location.origin,
  );
  if (connectionId) url.searchParams.set("connection_id", connectionId);
  return {
    url: url.toString(),
    // A function gives every reconnect the latest credential without putting
    // the bearer in browser history, proxy logs, or the request URL.
    headers: () => {
      const token = readStoredToken();
      return token ? { Authorization: `Bearer ${token}` } : {};
    },
  };
}

// --- Streaming (WebSocket) ---

// Same-origin enforcement happens at the composition layer. The
// browser sends Origin automatically; the bearer travels via the
// `?token=` URL parameter (the WS handshake API in browsers has no
// way to set a custom request header).
export function openEventSocket({ threadId }: ThreadOptions = {}) {
  const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = new URL(
    `${V2_BASE}/threads/${encodePathSegment(threadId)}/ws`,
    window.location.origin,
  );
  url.protocol = scheme;
  const token = readStoredToken();
  if (token) url.searchParams.set("token", token);
  return new WebSocket(url.toString());
}

// --- Run cancellation ---

export function cancelRun({
  threadId,
  runId,
  reason,
  clientActionId: clientId,
}: RunOptions & { reason?: string; clientActionId?: string } = {}) {
  const body: { client_action_id: string; reason?: string } = {
    client_action_id: clientId || clientActionId(),
  };
  if (reason) body.reason = reason;
  return apiFetch(
    `${V2_BASE}/threads/${encodePathSegment(threadId)}/runs/${encodePathSegment(runId)}/cancel`,
    {
      method: "POST",
      body: JSON.stringify(body),
    },
  );
}

// --- Gate resolution ---

// `resolution` is one of "approved" | "declined" | "credential_provided".
// `always` is only meaningful when `resolution === "approved"`.
// `credentialRef` is only meaningful when `resolution === "credential_provided"`.
export function resolveGate({
  threadId,
  runId,
  gateRef,
  resolution,
  always,
  credentialRef,
  clientActionId: clientId,
  signal,
}: RunOptions & {
  gateRef?: string;
  resolution?: "approved" | "declined" | "credential_provided";
  always?: boolean;
  credentialRef?: string;
  clientActionId?: string;
  signal?: AbortSignal;
} = {}) {
  const body: {
    client_action_id: string;
    resolution?: "approved" | "declined" | "credential_provided";
    always?: boolean;
    credential_ref?: string;
  } = {
    client_action_id: clientId || clientActionId(),
    resolution,
  };
  if (always != null) body.always = always;
  if (credentialRef) body.credential_ref = credentialRef;
  return apiFetch(
    `${V2_BASE}/threads/${encodePathSegment(threadId)}/runs/${encodePathSegment(runId)}/gates/${encodePathSegment(gateRef)}/resolve`,
    {
      method: "POST",
      signal,
      body: JSON.stringify(body),
    },
  );
}

// --- Product auth ---

export function submitManualToken({
  provider,
  accountLabel,
  token,
  threadId,
  runId,
  gateRef,
  signal,
}: RunOptions & {
  provider?: string;
  accountLabel?: string;
  token?: string;
  gateRef?: string;
  signal?: AbortSignal;
} = {}) {
  return apiFetch("/api/reborn/product-auth/manual-token/submit", {
    method: "POST",
    signal,
    body: JSON.stringify({
      provider,
      account_label: accountLabel,
      token,
      thread_id: threadId,
      run_id: runId,
      gate_ref: gateRef,
    }),
  });
}

// --- Extension setup ---

export function setupExtension(
  extensionName: string,
  {
    action,
    payload,
    clientActionId: clientId,
  }: { action?: string; payload?: unknown; clientActionId?: string } = {},
) {
  const body: {
    client_action_id: string;
    action?: string;
    payload?: unknown;
  } = { client_action_id: clientId || clientActionId() };
  if (action) body.action = action;
  if (payload !== undefined) body.payload = payload;
  return apiFetch(
    `${V2_BASE}/extensions/${encodeURIComponent(extensionName)}/setup`,
    {
      method: "POST",
      body: JSON.stringify(body),
    },
  );
}

// --- TODO stubs for v1-shaped helpers brought-back code still imports ---
//
// Issue #3886 Hard Non-Goal: the browser must not call legacy
// gateway routes without a v2 counterpart. The functions below
// preserve the fork's import surface so the admin/settings/extensions
// pages render, but they return empty/null data without sending any
// HTTP request. When a v2 equivalent lands, replace the stub body
// with the real wire call.

export function gatewayStatus() {
  // TODO: requires v2 gateway-status endpoint. Returning a zeroed
  // shape so any consumer reading `data.engine_v2_enabled`,
  // `data.llm_backend`, etc. resolves cleanly to falsey values.
  return Promise.resolve({
    engine_v2_enabled: false,
    restart_enabled: false,
    total_connections: null,
    llm_backend: null,
    llm_model: null,
    todo: true,
  });
}

// --- v2 auth surface ---
//
// The host mounts `webui_v2_auth_router` from
// `ironclaw_webui` at the same origin as the SPA. The
// providers endpoint is public; the login + callback routes are
// reached via `<a href>` navigations from the login page (the SPA
// does not invoke them via fetch). The callback redirects back with
// a short-lived `login_ticket`; the SPA exchanges it over same-origin
// JSON for the real bearer. Logout sends the current bearer so the
// server-side session can be revoked.

export async function fetchAuthProviders() {
  // Unauthenticated GET — the server returns `{ providers: [] }`
  // when nothing is configured. Network failures collapse to an
  // empty list so a broken backend hides OAuth buttons rather than
  // surfacing a stack trace on the login page.
  try {
    const response = await fetch("/auth/providers", {
      headers: { Accept: "application/json" },
      credentials: "same-origin",
    });
    if (!response.ok) return { providers: [] };
    const data = await response.json();
    return {
      providers: Array.isArray(data?.providers) ? data.providers : [],
    };
  } catch (_) {
    // silent-ok: login UI fail-safe — a broken /auth/providers hides
    // OAuth buttons rather than breaking the login page, which still
    // accepts manual token paste.
    return { providers: [] };
  }
}

export async function exchangeLoginTicket(ticket: string) {
  const response = await fetch("/auth/session/exchange", {
    method: "POST",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
    },
    credentials: "same-origin",
    body: JSON.stringify({ ticket }),
  });
  if (!response.ok) {
    throw new ApiError("Could not complete sign-in.", {
      status: response.status,
      statusText: response.statusText,
      headers: response.headers,
    });
  }
  const data = await response.json();
  const token = (data?.token || "").trim();
  if (!token) {
    throw new ApiError("Sign-in response did not include a token.", {
      status: response.status,
      statusText: response.statusText,
      headers: response.headers,
      payload: data,
    });
  }
  return token;
}

export async function logout() {
  const token = readStoredToken();
  if (!token) return;
  const headers = new Headers({ Accept: "application/json" });
  headers.set("Authorization", `Bearer ${token}`);
  try {
    await fetch("/auth/logout", {
      method: "POST",
      headers,
      credentials: "same-origin",
    });
  } catch (_) {
    // Network failure should not block the SPA's local sign-out —
    // the caller still clears sessionStorage. Server-side cleanup
    // will eventually expire the session.
  }
}
