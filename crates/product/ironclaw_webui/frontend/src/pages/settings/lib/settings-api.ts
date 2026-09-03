import { apiFetch, clientActionId, type ApiRecord } from "../../../lib/api";

const OPERATOR_CONFIG_BASE = "/api/webchat/v2/operator/config";
const SETTINGS_TOOLS_BASE = "/api/webchat/v2/settings/tools";
const AUTO_APPROVE_KEY = "agent.auto_approve_tools";
const TOOL_PREFIX = "tool.";
const TOOL_PERMISSION_UPDATE_TIMEOUT_MS = 30_000;
const TOOL_PERMISSION_STATES = new Set(["always_allow", "ask_each_time", "disabled"]);
const TOOL_PERMISSION_UPDATE_STATES = new Set([
  "default",
  "always_allow",
  "ask_each_time",
  "disabled",
]);

function recordField(
  response: ApiRecord,
  field: string,
  responseName: string,
): ApiRecord {
  const value = response[field];
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value as ApiRecord;
}

function recordArrayField(
  response: ApiRecord,
  field: string,
  responseName: string,
): ApiRecord[] {
  const value = response[field];
  if (
    !Array.isArray(value) ||
    !value.every(
      (entry) => typeof entry === "object" && entry !== null && !Array.isArray(entry),
    )
  ) {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value as ApiRecord[];
}

function unknownArrayField(
  response: ApiRecord,
  field: string,
  responseName: string,
): unknown[] {
  const value = response[field];
  if (!Array.isArray(value)) {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value;
}

function optionalUnknownArrayField(
  response: ApiRecord,
  field: string,
  responseName: string,
): unknown[] {
  return response[field] === undefined
    ? []
    : unknownArrayField(response, field, responseName);
}

function normalizeToolState(state) {
  if (state === "ask") return "ask_each_time";
  return TOOL_PERMISSION_STATES.has(state) ? state : "ask_each_time";
}

function normalizeToolUpdateState(state) {
  if (state === "ask") return "ask_each_time";
  return TOOL_PERMISSION_UPDATE_STATES.has(state) ? state : "default";
}

function normalizeEffectiveSource(source) {
  return ["default", "global", "override"].includes(source) ? source : "default";
}

function persistedToolFromConfigEntry(entry, expectedName, requestedState) {
  const value = entry?.value;
  const hasPersistedShape =
    entry?.key === `${TOOL_PREFIX}${expectedName}` &&
    value != null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    value.name === expectedName &&
    TOOL_PERMISSION_STATES.has(value.state) &&
    TOOL_PERMISSION_STATES.has(value.default_state) &&
    typeof value.locked === "boolean" &&
    ["default", "global", "override", "locked"].includes(value.effective_source) &&
    entry.source === value.effective_source &&
    typeof entry.mutable === "boolean";
  if (!hasPersistedShape) {
    throw new Error("Permission save response is missing a valid persisted tool entry");
  }

  const tool = toolFromConfigEntry(entry);
  const confirmsRequestedState =
    requestedState === "default"
      ? value.effective_source !== "override"
      : tool?.state === requestedState;
  if (!tool || !confirmsRequestedState) {
    throw new Error("Permission save response did not confirm the requested tool state");
  }
  return tool;
}

export function toolFromConfigEntry(entry) {
  if (!entry?.key?.startsWith(TOOL_PREFIX)) return null;
  const value = entry.value || {};
  const name = value.name || entry.key.slice(TOOL_PREFIX.length);
  return {
    name,
    description: value.description || "",
    state: normalizeToolState(value.state),
    default_state: normalizeToolState(value.default_state),
    locked: Boolean(value.locked || entry.mutable === false),
    effective_source: normalizeEffectiveSource(value.effective_source || entry.source),
  };
}

export function settingsFromOperatorConfig(data) {
  const settings = {};
  for (const entry of data.entries || []) {
    if (entry?.key === AUTO_APPROVE_KEY) {
      settings[AUTO_APPROVE_KEY] = Boolean(entry.value);
    }
  }
  return settings;
}

export async function fetchSettingsExport() {
  const data = await apiFetch(SETTINGS_TOOLS_BASE);
  return {
    settings: settingsFromOperatorConfig(data),
    diagnostics: optionalUnknownArrayField(data, "diagnostics", "tool settings"),
    precedence: optionalUnknownArrayField(data, "precedence", "tool settings"),
  };
}
export async function fetchSetting(key) {
  if (key === AUTO_APPROVE_KEY) {
    const data = await fetchSettingsExport();
    // Default ON when unset, mirroring backend AUTO_APPROVE_DEFAULT_ENABLED.
    return data.settings[AUTO_APPROVE_KEY] ?? true;
  }
  const data = await apiFetch(`${OPERATOR_CONFIG_BASE}/${encodeURIComponent(key)}`);
  return recordField(data, "entry", "operator config").value ?? null;
}

type SettingEntry = {
  key: string;
  value: unknown;
  mutable?: boolean;
  source?: string;
};

export type SettingUpdateSuccess = {
  success: true;
  entry: SettingEntry;
  value: unknown;
};

export type SettingUpdateFailure = {
  success: false;
  message?: string;
};

export type SettingUpdateResult = SettingUpdateSuccess | SettingUpdateFailure;

function settingUpdateResult(data: unknown, expectedKey: string): SettingUpdateResult {
  if (typeof data !== "object" || data === null || Array.isArray(data)) {
    throw new Error("Save response is not an object");
  }
  const success = Reflect.get(data, "success");
  if (success === false) {
    const message = Reflect.get(data, "message");
    return {
      success: false,
      ...(typeof message === "string" ? { message } : {}),
    };
  }
  if (success !== undefined && success !== true) {
    throw new Error("Save response has an invalid success flag");
  }

  const entry = Reflect.get(data, "entry");
  if (
    typeof entry !== "object" ||
    entry === null ||
    Array.isArray(entry) ||
    Reflect.get(entry, "key") !== expectedKey ||
    !Object.prototype.hasOwnProperty.call(entry, "value")
  ) {
    throw new Error("Save response is missing the confirmed setting entry");
  }

  const mutable = Reflect.get(entry, "mutable");
  const source = Reflect.get(entry, "source");
  const confirmedEntry: SettingEntry = {
    key: expectedKey,
    value: Reflect.get(entry, "value"),
    ...(typeof mutable === "boolean" ? { mutable } : {}),
    ...(typeof source === "string" ? { source } : {}),
  };
  return { success: true, entry: confirmedEntry, value: confirmedEntry.value };
}

export async function updateSetting(
  key: string,
  value: unknown,
): Promise<SettingUpdateResult> {
  if (key === AUTO_APPROVE_KEY) {
    const data = await apiFetch(SETTINGS_TOOLS_BASE, {
      method: "POST",
      body: JSON.stringify({ enabled: Boolean(value) }),
    });
    return settingUpdateResult(data, key);
  }
  const data = await apiFetch(`${OPERATOR_CONFIG_BASE}/${encodeURIComponent(key)}`, {
    method: "POST",
    body: JSON.stringify({ value }),
  });
  return settingUpdateResult(data, key);
}

type SettingsImportUpdateResult = Awaited<ReturnType<typeof updateSetting>>;

export type SettingsImportSuccess = {
  success: true;
  imported: number;
  results: SettingsImportUpdateResult[];
};

export type NoSupportedSettingsImportFailure = {
  success: false;
  imported: 0;
  results: SettingsImportUpdateResult[];
  message: string;
};

export type SettingsImportResult =
  | SettingsImportSuccess
  | NoSupportedSettingsImportFailure;

export class NoSupportedSettingsImportError extends Error {
  constructor(failure: NoSupportedSettingsImportFailure) {
    super(failure.message);
    this.name = "NoSupportedSettingsImportError";
  }
}

export async function importSettings(
  payload: { settings?: Record<string, unknown> } | null | undefined
): Promise<SettingsImportResult> {
  const settings = payload?.settings || {};
  const imported: SettingsImportUpdateResult[] = [];
  if (Object.prototype.hasOwnProperty.call(settings, AUTO_APPROVE_KEY)) {
    const result = await updateSetting(
      AUTO_APPROVE_KEY,
      Boolean(settings[AUTO_APPROVE_KEY]),
    );
    imported.push(result);
    if (result.success === false) {
      return {
        success: false,
        imported: 0,
        results: imported,
        message: result.message || "The setting could not be saved",
      };
    }
  }
  if (imported.length === 0) {
    return {
      success: false,
      imported: 0,
      results: imported,
      message: "No supported settings were found in the selected file",
    };
  }
  return { success: true, imported: imported.length, results: imported };
}
// LLM provider configuration — v2 native endpoints. The snapshot is the single
// source of truth: a unified provider list (built-in + operator-defined) plus
// the active selection. API-key values are write-only; the snapshot only ever
// reports `api_key_set`.
export interface UserModelPolicyResponse extends ApiRecord {
  provider_id?: string;
  allowed_models: string[];
  model_entries: ApiRecord[];
}

export interface LlmProvidersResponse extends ApiRecord {
  providers: ApiRecord[];
  active: ApiRecord | null;
  user_model_policy: UserModelPolicyResponse | null;
}

function decodeUserModelPolicy(value: unknown): UserModelPolicyResponse | null {
  if (value === undefined || value === null) return null;
  if (typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("invalid LLM providers response");
  }
  const policy = value as ApiRecord;
  if (
    policy.provider_id !== undefined &&
    typeof policy.provider_id !== "string"
  ) {
    throw new TypeError("invalid LLM providers response");
  }
  const allowedModels = unknownArrayField(
    policy,
    "allowed_models",
    "LLM providers",
  );
  if (!allowedModels.every((model) => typeof model === "string")) {
    throw new TypeError("invalid LLM providers response");
  }
  return {
    ...policy,
    allowed_models: allowedModels,
    model_entries: recordArrayField(
      policy,
      "model_entries",
      "LLM providers",
    ),
  } as UserModelPolicyResponse;
}

export async function fetchLlmProviders(): Promise<LlmProvidersResponse> {
  const response = await apiFetch("/api/webchat/v2/llm/providers");
  const active =
    response.active === undefined || response.active === null
      ? null
      : recordField(response, "active", "LLM providers");
  return {
    ...response,
    providers: recordArrayField(response, "providers", "LLM providers"),
    active,
    user_model_policy: decodeUserModelPolicy(response.user_model_policy),
  };
}
export function fetchUserModelCatalog() {
  return apiFetch("/api/webchat/v2/llm/models");
}
export function fetchUserModelPreference() {
  return apiFetch("/api/webchat/v2/llm/model-preference");
}
export function setUserModelPolicy(payload) {
  return apiFetch("/api/webchat/v2/llm/model-policy", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}
export function setUserModelPreference(model) {
  return apiFetch("/api/webchat/v2/llm/model-preference", {
    method: "PUT",
    body: JSON.stringify({ model }),
  });
}
export function upsertLlmProvider(payload) {
  const { clientActionId: callerClientActionId, ...request } = payload;
  return apiFetch("/api/webchat/v2/llm/providers", {
    method: "POST",
    body: JSON.stringify({
      ...request,
      client_action_id:
        request.client_action_id || callerClientActionId || clientActionId(),
    }),
  });
}
export function deleteLlmProvider(providerId) {
  return apiFetch(`/api/webchat/v2/llm/providers/${encodeURIComponent(providerId)}/delete`, {
    method: "POST",
  });
}
export function setActiveLlm(payload) {
  return apiFetch("/api/webchat/v2/llm/active", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}
export function testLlmProviderConnection(payload) {
  return apiFetch("/api/webchat/v2/llm/test-connection", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}
export function listLlmProviderModels(payload) {
  return apiFetch("/api/webchat/v2/llm/list-models", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}
// Begin NEAR AI browser login. Returns { auth_url } to open; a background task
// stores the session token and makes NEAR AI active once the user authorizes.
export function startNearaiLogin(payload) {
  return apiFetch("/api/webchat/v2/llm/nearai/login", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

// Complete a NEAR AI wallet (NEP-413) login. `payload` carries the browser
// wallet's signed message; the backend relays it to NEAR AI, stores the session
// token, and makes NEAR AI active. Returns { active }.
export function completeNearaiWalletLogin(payload) {
  return apiFetch("/api/webchat/v2/llm/nearai/wallet", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

// Begin an OpenAI Codex (ChatGPT subscription) device-code login. Returns
// { user_code, verification_uri } to display; a background task polls for
// authorization, stores the tokens, and makes Codex active once authorized.
export function startCodexLogin() {
  return apiFetch("/api/webchat/v2/llm/codex/login", {
    method: "POST",
  });
}
export async function fetchTools() {
  const data = await apiFetch(SETTINGS_TOOLS_BASE);
  return {
    tools: recordArrayField(data, "entries", "tool settings")
      .map(toolFromConfigEntry)
      .filter(Boolean),
    diagnostics: optionalUnknownArrayField(data, "diagnostics", "tool settings"),
  };
}
export async function updateToolPermission(name, state) {
  const normalized = normalizeToolUpdateState(state);
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), TOOL_PERMISSION_UPDATE_TIMEOUT_MS);
  try {
    const data = await apiFetch(`${SETTINGS_TOOLS_BASE}/${encodeURIComponent(name)}`, {
      method: "POST",
      body: JSON.stringify({ state: normalized }),
      signal: controller.signal,
    });
    const tool = persistedToolFromConfigEntry(data.entry, name, normalized);
    const entry = recordField(data, "entry", "tool permission update");
    return { success: true, tool, entry };
  } finally {
    clearTimeout(timeoutId);
  }
}
export async function fetchExtensions() {
  const response = await apiFetch("/api/webchat/v2/extensions");
  return {
    ...response,
    extensions: recordArrayField(response, "extensions", "extensions"),
  };
}
export async function fetchExtensionRegistry() {
  const response = await apiFetch("/api/webchat/v2/extensions/registry");
  return {
    ...response,
    entries: recordArrayField(response, "entries", "extension registry"),
  };
}
export function fetchSkills() {
  return apiFetch("/api/webchat/v2/skills");
}
export function fetchSkillContent(name) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`);
}
export function installSkill(payload) {
  return apiFetch("/api/webchat/v2/skills/install", {
    method: "POST",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify(payload),
  });
}
export function updateSkill(name, payload) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`, {
    method: "PUT",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify(payload),
  });
}
export function removeSkill(name) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`, {
    method: "DELETE",
    headers: { "X-Confirm-Action": "true" },
  });
}
export function setSkillAutoActivate(name, enabled) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}/auto-activate`, {
    method: "POST",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify({ enabled }),
  });
}
// Global "auto-activate learned skills" master switch. When disabled, learned
// skills activate only via an explicit /name mention.
export function setAutoActivateLearned(enabled) {
  return apiFetch(`/api/webchat/v2/skills/auto-activate-learned`, {
    method: "POST",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify({ enabled }),
  });
}
// Trace Commons credits — read-only, scoped server-side to the
// authenticated caller. The response is the contributor-local view as
// of the last credit sync; the authoritative ledger is server-side.
export interface TraceCreditsResponse extends ApiRecord {
  enrolled: boolean;
  submissions_total: number;
  submissions_submitted: number;
  submissions_accepted: number;
  manual_review_hold_count: number;
  recent_explanations: string[];
  holds: TraceHoldResponse[];
}

interface TraceHoldResponse extends ApiRecord {
  submission_id: string;
  reason: string;
}

interface AccountTraceResponse extends ApiRecord {
  submission_id: string;
  status: string;
}

function decodeTraceHolds(value: unknown): TraceHoldResponse[] {
  if (!Array.isArray(value)) {
    throw new TypeError("invalid trace credits response");
  }
  return value.map((entry) => {
    if (
      typeof entry !== "object" ||
      entry === null ||
      Array.isArray(entry) ||
      !("submission_id" in entry) ||
      typeof entry.submission_id !== "string" ||
      !("reason" in entry) ||
      typeof entry.reason !== "string"
    ) {
      throw new TypeError("invalid trace credits response");
    }
    return entry as TraceHoldResponse;
  });
}

function numberField(
  response: ApiRecord,
  field: string,
  responseName: string,
): number {
  const value = response[field];
  if (typeof value !== "number") {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value;
}

export async function fetchTraceCredits(): Promise<TraceCreditsResponse> {
  const response = await apiFetch("/api/webchat/v2/traces/credit");
  if (typeof response.enrolled !== "boolean") {
    throw new TypeError("invalid trace credits response");
  }
  const recentExplanations = optionalUnknownArrayField(
    response,
    "recent_explanations",
    "trace credits",
  );
  if (!recentExplanations.every((line) => typeof line === "string")) {
    throw new TypeError("invalid trace credits response");
  }
  return {
    ...response,
    enrolled: response.enrolled,
    submissions_total: numberField(
      response,
      "submissions_total",
      "trace credits",
    ),
    submissions_submitted: numberField(
      response,
      "submissions_submitted",
      "trace credits",
    ),
    submissions_accepted: numberField(
      response,
      "submissions_accepted",
      "trace credits",
    ),
    manual_review_hold_count: numberField(
      response,
      "manual_review_hold_count",
      "trace credits",
    ),
    recent_explanations: recentExplanations,
    holds: decodeTraceHolds(
      optionalUnknownArrayField(response, "holds", "trace credits"),
    ),
  };
}
// Submitted Trace Commons traces for the authenticated caller (read-only,
// server-scoped). Mirrors fetchTraceCredits.
export async function fetchAccountTraces() {
  const response = await apiFetch("/api/webchat/v2/traces/account");
  if (typeof response.enrolled !== "boolean") {
    throw new TypeError("invalid account traces response");
  }
  const traces = recordArrayField(response, "traces", "account traces").map(
    (trace): AccountTraceResponse => {
      if (
        typeof trace.submission_id !== "string" ||
        typeof trace.status !== "string"
      ) {
        throw new TypeError("invalid account traces response");
      }
      return trace as AccountTraceResponse;
    },
  );
  return {
    ...response,
    enrolled: response.enrolled,
    traces,
  };
}
// Mint a one-time Trace Commons browser login link for the authenticated
// caller. The returned URL is a single-use account credential delivered only
// over this authenticated response — open it immediately, never log or store
// it. Unenrolled callers get { minted: false, enrolled: false }.
export function mintAccountLoginLink() {
  return apiFetch("/api/webchat/v2/traces/account-login-link", { method: "POST" });
}
// Authorize a held (manual-review) trace for submission. No request body —
// the submission id is in the path. Returns { authorized: bool }.
export function authorizeTraceHold(submissionId) {
  return apiFetch(
    `/api/webchat/v2/traces/holds/${encodeURIComponent(submissionId)}/authorize`,
    { method: "POST" }
  );
}
export function fetchUsers() {
  return Promise.resolve({ users: [], todo: true });
}
export function createUser(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 users endpoint" });
}
export function updateUser(_id, _payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 users endpoint" });
}
