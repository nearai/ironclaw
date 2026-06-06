// Extensions surface:
// - The browser talks only to `/api/webchat/v2/extensions/*` endpoints.
// - The v2 backend owns the registry/list/install/activate/remove/setup
//   projection and maps those operations to the extension registry.

import { apiFetch, setupExtension } from "../../../lib/api.js";

export function fetchExtensions() {
  return apiFetch("/api/webchat/v2/extensions");
}
export function fetchExtensionRegistry() {
  return apiFetch("/api/webchat/v2/extensions/registry");
}
export function installExtension(packageRef) {
  return apiFetch("/api/webchat/v2/extensions/install", {
    method: "POST",
    body: JSON.stringify({ package_ref: packageRef }),
  });
}
export function activateExtension(packageRef) {
  return apiFetch(`/api/webchat/v2/extensions/${encodeURIComponent(packageId(packageRef))}/activate`, {
    method: "POST",
  });
}
export function removeExtension(packageRef) {
  return apiFetch(`/api/webchat/v2/extensions/${encodeURIComponent(packageId(packageRef))}/remove`, {
    method: "POST",
  });
}
export function fetchExtensionSetup(packageRef) {
  return apiFetch(`/api/webchat/v2/extensions/${encodeURIComponent(packageId(packageRef))}/setup`);
}
export function submitExtensionSetup(packageRef, secrets, fields) {
  return setupExtension(packageId(packageRef), {
    action: "submit",
    payload: { secrets, fields },
  });
}
export function startExtensionOauth(packageRef, secret) {
  const setup = secret?.setup || {};
  const expiresAt = new Date(Date.now() + 10 * 60 * 1000).toISOString();
  return apiFetch(
    `/api/webchat/v2/extensions/${encodeURIComponent(packageId(packageRef))}/setup/oauth/start`,
    {
      method: "POST",
      body: JSON.stringify({
        provider: secret.provider,
        account_label: setup.account_label || `${secret.provider} credential`,
        scopes: setup.scopes || [],
        expires_at: expiresAt,
        invocation_id: setup.invocation_id,
      }),
    }
  );
}
export function fetchPairingRequests() {
  return Promise.resolve({ requests: [] });
}
export function approvePairingCode() {
  return Promise.resolve({
    success: false,
    message: "Pairing requires a v2 pairing endpoint.",
  });
}

function packageId(packageRef) {
  const id = typeof packageRef === "string" ? packageRef : packageRef?.id;
  if (!id) {
    throw new Error("Extension package_ref is required");
  }
  return id;
}
