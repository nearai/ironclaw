import { apiFetch } from "../../../lib/api.js";

export function fetchExtensions() {
  return apiFetch("/api/extensions");
}

export function fetchExtensionRegistry() {
  return apiFetch("/api/extensions/registry");
}

export function installExtension(name, kind) {
  return apiFetch("/api/extensions/install", {
    method: "POST",
    body: JSON.stringify({ name, kind }),
  });
}

export function activateExtension(name) {
  return apiFetch(`/api/extensions/${encodeURIComponent(name)}/activate`, {
    method: "POST",
  });
}

export function removeExtension(name) {
  return apiFetch(`/api/extensions/${encodeURIComponent(name)}/remove`, {
    method: "POST",
  });
}

export function fetchExtensionSetup(name) {
  return apiFetch(`/api/extensions/${encodeURIComponent(name)}/setup`);
}

export function submitExtensionSetup(name, secrets, fields) {
  return apiFetch(`/api/extensions/${encodeURIComponent(name)}/setup`, {
    method: "POST",
    body: JSON.stringify({ secrets: secrets || {}, fields: fields || {} }),
  });
}

export function fetchPairingRequests(channel) {
  return apiFetch(`/api/pairing/${encodeURIComponent(channel)}`);
}

export function approvePairingCode(channel, code) {
  return apiFetch(`/api/pairing/${encodeURIComponent(channel)}/approve`, {
    method: "POST",
    body: JSON.stringify({ code }),
  });
}
