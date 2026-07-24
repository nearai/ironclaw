// @ts-nocheck
import { apiFetch } from "./api";
import { channelSetupError } from "./channel-setup-api";

// Generic WebGeneratedCode pairing endpoints (extension-runtime §5.5): the
// backend registers a pairing service per extension whose account-setup
// descriptor declares the `web_generated_code` connect strategy. Presentation
// routes directly from the manifest strategy; these calls never probe support.
export function extensionPairingPath(extensionId, action) {
  return `/api/webchat/v2/extensions/${encodeURIComponent(extensionId)}/pairing/${action}`;
}

// -> { code, deep_link?, expires_at }; mints (or rotates) the caller's code.
export function mintExtensionPairingCode(extensionId) {
  return apiFetch(extensionPairingPath(extensionId, "mint"), { method: "POST" });
}

// -> { connected, pending: { code, deep_link?, expires_at } | null }
export function getExtensionPairingStatus(extensionId) {
  return apiFetch(extensionPairingPath(extensionId, "status"));
}

// -> 204; unpairs the caller's account on this channel.
export function unpairExtension(extensionId) {
  return apiFetch(extensionPairingPath(extensionId, "unpair"), { method: "POST" });
}

export const extensionPairingError = channelSetupError;
