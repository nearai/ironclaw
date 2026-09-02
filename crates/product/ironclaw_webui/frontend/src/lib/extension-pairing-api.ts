import { apiFetch } from "./api";
import { channelSetupError } from "./channel-setup-api";

// Generic WebGeneratedCode pairing endpoints (extension-runtime §5.5): the
// backend registers a pairing service per extension whose account-setup
// descriptor declares the `web_generated_code` connect strategy. Presentation
// routes directly from the manifest strategy; these calls never probe support.
export function extensionPairingPath(extensionId: string, action: string): string {
  return `/api/webchat/v2/extensions/${encodeURIComponent(extensionId)}/pairing/${action}`;
}

export interface ExtensionPairingCode {
  code: string;
  deep_link?: string | null;
  expires_at: string;
}

export interface ExtensionPairingStatus {
  connected: boolean;
  pending: ExtensionPairingCode | null;
}

// -> { code, deep_link?, expires_at }; mints (or rotates) the caller's code.
export function mintExtensionPairingCode(
  extensionId: string,
): Promise<ExtensionPairingCode> {
  return apiFetch(extensionPairingPath(extensionId, "mint"), {
    method: "POST",
  }) as Promise<ExtensionPairingCode>;
}

// -> { connected, pending: { code, deep_link?, expires_at } | null }
export function getExtensionPairingStatus(
  extensionId: string,
): Promise<ExtensionPairingStatus> {
  return apiFetch(
    extensionPairingPath(extensionId, "status"),
  ) as Promise<ExtensionPairingStatus>;
}

// -> 204; unpairs the caller's account on this channel.
export function unpairExtension(extensionId: string) {
  return apiFetch(extensionPairingPath(extensionId, "unpair"), { method: "POST" });
}

export const extensionPairingError = channelSetupError;
