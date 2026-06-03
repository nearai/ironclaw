import { apiFetch } from "../../../lib/api.js";

// Settings endpoints depend on v1 `/api/settings/*`, `/api/llm/*`,
// `/api/tools/*`, `/api/skills/*`, etc. Extension reads use the v2
// registry/list endpoints; the remaining settings APIs are TODO stubs.

export function fetchSettingsExport() {
  return Promise.resolve({ settings: {}, todo: true });
}
export function fetchSetting(_key) {
  return Promise.resolve(null);
}
export function updateSetting(_key, _value) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 settings endpoint" });
}
export function importSettings(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 settings endpoint" });
}
export function fetchLlmProviders() {
  return Promise.resolve({ providers: [], custom_providers: [], builtin_overrides: {}, todo: true });
}
export function testLlmProviderConnection(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 LLM endpoint" });
}
export function listLlmProviderModels(_payload) {
  return Promise.resolve({ models: [], todo: true });
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
