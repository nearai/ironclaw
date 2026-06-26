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
export function fetchPairingRequests(channel) {
  return apiFetch(`/api/pairing/${encodeURIComponent(channelName(channel))}`);
}
export function approvePairingCode(channel, code, options = {}) {
  const body = { code };
  if (options.threadId) body.thread_id = options.threadId;
  if (options.requestId) body.request_id = options.requestId;
  return apiFetch(`/api/pairing/${encodeURIComponent(channelName(channel))}/approve`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

function channelName(channel) {
  if (typeof channel !== "string" || !channel.trim()) {
    throw new Error("Pairing channel is required");
  }
  return channel.trim();
}

function packageId(packageRef) {
  const id = typeof packageRef === "string" ? packageRef : packageRef?.id;
  if (!id) {
    throw new Error("Extension package_ref is required");
  }
  return id;
}
