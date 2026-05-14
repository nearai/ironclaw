import { apiFetch } from "../../../lib/api.js";

export function fetchSettingsExport() {
  return apiFetch("/api/settings/export");
}

export function fetchSetting(key) {
  return apiFetch(`/api/settings/${encodeURIComponent(key)}`);
}

export function updateSetting(key, value) {
  if (value === null || value === undefined) {
    return apiFetch(`/api/settings/${encodeURIComponent(key)}`, { method: "DELETE" });
  }
  return apiFetch(`/api/settings/${encodeURIComponent(key)}`, {
    method: "PUT",
    body: JSON.stringify({ value }),
  });
}

export function importSettings(payload) {
  return apiFetch("/api/settings/import", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function fetchLlmProviders() {
  return apiFetch("/api/llm/providers");
}

export function testLlmProviderConnection(payload) {
  return apiFetch("/api/llm/test_connection", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listLlmProviderModels(payload) {
  return apiFetch("/api/llm/list_models", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function fetchTools() {
  return apiFetch("/api/settings/tools");
}

export function updateToolPermission(name, state) {
  return apiFetch(`/api/settings/tools/${encodeURIComponent(name)}`, {
    method: "PUT",
    body: JSON.stringify({ state }),
  });
}

export function fetchExtensions() {
  return apiFetch("/api/extensions");
}

export function fetchExtensionRegistry() {
  return apiFetch("/api/extensions/registry");
}

export function fetchSkills() {
  return apiFetch("/api/skills");
}

export function installSkill(payload) {
  return apiFetch("/api/skills/install", {
    method: "POST",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify(payload),
  });
}

export function removeSkill(name) {
  return apiFetch(`/api/skills/${encodeURIComponent(name)}`, {
    method: "DELETE",
    headers: { "X-Confirm-Action": "true" },
  });
}

export function fetchUsers() {
  return apiFetch("/api/admin/users");
}

export function createUser(payload) {
  return apiFetch("/api/admin/users", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function updateUser(id, payload) {
  return apiFetch(`/api/admin/users/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  });
}
