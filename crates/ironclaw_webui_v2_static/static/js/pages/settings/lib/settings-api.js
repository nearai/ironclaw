import { apiFetch } from "../../../lib/api.js";

// Settings endpoints depend on v1 `/api/settings/*`, `/api/tools/*`, etc.
// LLM, extension, and skills reads use v2 endpoints. Remaining settings APIs
// are TODO stubs.

// Operator config (#4776): the v2 operator-config channel backs the persisted
// settings the WebUI can write today. `list` projects the wired keys (currently
// `agent.auto_approve_tools`) into the flat `{ settings: { key: value } }` shape
// the settings page consumes; per-key get/set ride the `{key}` route.
export async function fetchSettingsExport() {
  const response = await apiFetch("/api/webchat/v2/operator/config");
  const settings = {};
  for (const entry of response?.entries || []) {
    settings[entry.key] = entry.value;
  }
  return { settings };
}
export async function fetchSetting(key) {
  const response = await apiFetch(
    `/api/webchat/v2/operator/config/${encodeURIComponent(key)}`
  );
  return response?.entry?.value ?? null;
}
export function updateSetting(key, value) {
  return apiFetch(`/api/webchat/v2/operator/config/${encodeURIComponent(key)}`, {
    method: "POST",
    body: JSON.stringify({ value }),
  });
}
export function importSettings(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 settings endpoint" });
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
export function fetchTools() {
  return Promise.resolve({ tools: [], todo: true });
}
export function updateToolPermission(_name, _state) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 tools endpoint" });
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
// Trace Commons credits — read-only, scoped server-side to the
// authenticated caller. The response is the contributor-local view as
// of the last credit sync; the authoritative ledger is server-side.
export function fetchTraceCredits() {
  return apiFetch("/api/webchat/v2/traces/credit");
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
