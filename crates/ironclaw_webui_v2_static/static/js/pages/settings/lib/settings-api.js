import { apiFetch } from "../../../lib/api.js";

const TOOL_SETTINGS_PATH = "/api/webchat/v2/settings/tools";
const AUTO_APPROVE_TOOLS_KEY = "agent.auto_approve_tools";

// Settings endpoints depend on v1 `/api/settings/*`, `/api/llm/*`,
// `/api/tools/*`, `/api/skills/*`, etc. Extension reads use the v2
// registry/list endpoints; the remaining settings APIs are TODO stubs.

export async function fetchSettingsExport() {
  const tools = await apiFetch(TOOL_SETTINGS_PATH);
  return { settings: tools.settings || {}, todo: false };
}
export function fetchSetting(_key) {
  return Promise.resolve(null);
}
export function updateSetting(key, value) {
  if (key === AUTO_APPROVE_TOOLS_KEY) {
    return apiFetch(TOOL_SETTINGS_PATH, {
      method: "POST",
      body: JSON.stringify({ enabled: Boolean(value) }),
    });
  }
  return Promise.resolve({ success: false, message: "TODO: requires v2 settings endpoint" });
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
  return apiFetch(TOOL_SETTINGS_PATH);
}
export async function updateToolPermission(name, state) {
  const response = await apiFetch(`${TOOL_SETTINGS_PATH}/${encodeURIComponent(name)}`, {
    method: "POST",
    body: JSON.stringify({ state }),
  });
  return { success: true, ...response };
}
export function fetchExtensions() {
  return apiFetch("/api/webchat/v2/extensions");
}
export function fetchExtensionRegistry() {
  return apiFetch("/api/webchat/v2/extensions/registry");
}
export function fetchSkills() {
  return Promise.resolve({ skills: [], todo: true });
}
export function installSkill(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 skills endpoint" });
}
export function removeSkill(_name) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 skills endpoint" });
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
