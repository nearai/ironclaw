// Extensions surface:
// - The browser talks only to `/api/webchat/v2/extensions/*` endpoints.
// - The v2 backend owns the registry/list/install/remove/setup
//   projection and maps those operations to the extension registry.

import { apiFetch, clientActionId, setupExtension } from "../../../lib/api";

const OAUTH_START_TTL_MS = 5 * 60 * 1000;
type ExtensionMutationOptions = { clientActionId?: string };

export function fetchExtensions() {
  return apiFetch("/api/webchat/v2/extensions");
}
export function fetchExtensionRegistry() {
  return apiFetch("/api/webchat/v2/extensions/registry");
}
export function installExtension(packageRef, options: ExtensionMutationOptions = {}) {
  const clientId = options?.clientActionId;
  return apiFetch("/api/webchat/v2/extensions/install", {
    method: "POST",
    body: JSON.stringify({
      package_ref: packageRef,
      client_action_id: clientId || clientActionId(),
    }),
  });
}
export function removeExtension(packageRef, options: ExtensionMutationOptions = {}) {
  const clientId = options?.clientActionId;
  return apiFetch(
    `/api/webchat/v2/extensions/${encodeURIComponent(packageId(packageRef))}/remove`,
    {
      method: "POST",
      body: JSON.stringify({ client_action_id: clientId || clientActionId() }),
    }
  );
}
export function fetchExtensionSetup(packageRef) {
  return apiFetch(`/api/webchat/v2/extensions/${encodeURIComponent(packageId(packageRef))}/setup`);
}
export function submitExtensionSetup(
  packageRef,
  secrets,
  options: ExtensionMutationOptions = {},
) {
  const clientId = options?.clientActionId;
  return setupExtension(packageId(packageRef), {
    action: "submit",
    payload: { secrets },
    clientActionId: clientId,
  });
}
export function startExtensionOauth(packageRef, secret) {
  const setup = secret?.setup || {};
  const expiresAt = new Date(Date.now() + OAUTH_START_TTL_MS).toISOString();
  return apiFetch(
    `/api/webchat/v2/extensions/${encodeURIComponent(packageId(packageRef))}/setup/oauth/start`,
    {
      method: "POST",
      body: JSON.stringify({
        requirement: secret?.name,
        expires_at: expiresAt,
        invocation_id: setup.invocation_id,
      }),
    }
  );
}
// Origin-independent OAuth completion backstop. The same-origin
// localStorage/BroadcastChannel signal emitted by the callback page never
// reaches the opener when the callback runs on a different origin (local ngrok
// callback vs 127.0.0.1 opener, or split app/callback domains in prod). Polling
// the durable flow status by id closes that gap. `invocationId` is the id the
// start response minted (`callback_scope.invocation_id`); the caller-scoped
// backend needs it to locate its own flow. This is an explicit mutating
// reconciliation command: the separate GET status route remains observational,
// while this command may retry only the completed flow's unfenced internal
// continuation. It never repeats provider exchange or performs cleanup.
// Non-OK responses resolve to null so the watcher never throws.
export function fetchOauthFlowStatus(flowId, invocationId) {
  const query = invocationId
    ? `?invocation_id=${encodeURIComponent(invocationId)}`
    : "";
  return apiFetch(
    `/api/reborn/product-auth/oauth/flow/${encodeURIComponent(flowId)}/reconcile${query}`,
    { method: "POST" },
  ).catch(() => null);
}

export function importExtension(file) {
  return apiFetch("/api/webchat/v2/extensions/import", {
    method: "POST",
    headers: { "Content-Type": "application/zip" },
    body: file,
  });
}

function packageId(packageRef) {
  const id = typeof packageRef === "string" ? packageRef : packageRef?.id;
  if (!id) {
    throw new Error("Extension package_ref is required");
  }
  return id;
}
