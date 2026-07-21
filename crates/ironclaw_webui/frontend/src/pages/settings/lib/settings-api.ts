import { apiFetch } from "../../../lib/api";

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
    diagnostics: data.diagnostics || [],
    precedence: data.precedence || [],
  };
}
export async function fetchSetting(key) {
  if (key === AUTO_APPROVE_KEY) {
    const data = await fetchSettingsExport();
    // Default ON when unset, mirroring backend AUTO_APPROVE_DEFAULT_ENABLED.
    return data.settings[AUTO_APPROVE_KEY] ?? true;
  }
  const data = await apiFetch(`${OPERATOR_CONFIG_BASE}/${encodeURIComponent(key)}`);
  return data.entry?.value ?? null;
}
export async function updateSetting(key, value) {
  if (key === AUTO_APPROVE_KEY) {
    const data = await apiFetch(SETTINGS_TOOLS_BASE, {
      method: "POST",
      body: JSON.stringify({ enabled: Boolean(value) }),
    });
    return { success: true, entry: data.entry, value: data.entry?.value };
  }
  const data = await apiFetch(`${OPERATOR_CONFIG_BASE}/${encodeURIComponent(key)}`, {
    method: "POST",
    body: JSON.stringify({ value }),
  });
  return { success: true, entry: data.entry, value: data.entry?.value };
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
    imported.push(await updateSetting(AUTO_APPROVE_KEY, Boolean(settings[AUTO_APPROVE_KEY])));
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
export function fetchLlmProviders() {
  return apiFetch("/api/webchat/v2/llm/providers");
}
export function upsertLlmProvider(payload) {
  return apiFetch("/api/webchat/v2/llm/providers", {
    method: "POST",
    body: JSON.stringify(payload),
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
    tools: (data.entries || []).map(toolFromConfigEntry).filter(Boolean),
    diagnostics: data.diagnostics || [],
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
    const tool = persistedToolFromConfigEntry(data?.entry, name, normalized);
    return { success: true, tool, entry: data.entry };
  } finally {
    clearTimeout(timeoutId);
  }
}
export function fetchExtensions() {
  return apiFetch("/api/webchat/v2/extensions");
}
export function fetchExtensionRegistry() {
  return apiFetch("/api/webchat/v2/extensions/registry");
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
export function fetchTraceCredits() {
  return apiFetch("/api/webchat/v2/traces/credit");
}
// Submitted Trace Commons traces for the authenticated caller (read-only,
// server-scoped). Mirrors fetchTraceCredits.
export function fetchAccountTraces() {
  return apiFetch("/api/webchat/v2/traces/account");
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
