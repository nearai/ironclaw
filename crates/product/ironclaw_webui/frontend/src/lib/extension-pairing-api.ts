import { apiFetch } from "./api";
import { channelSetupError } from "./channel-setup-api";

// Generic WebGeneratedCode pairing endpoints (extension-runtime §5.5): the
// backend registers a pairing service per extension whose account-setup
// descriptor declares the `web_generated_code` connect strategy. Presentation
// routes directly from the manifest strategy; these calls never probe support.
export function extensionPairingPath(extensionId: string, action: string): string {
  if (!extensionId) throw new Error("extensionId is required");
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function decodePairingCode(
  value: unknown,
  responseName = "pairing code",
): ExtensionPairingCode {
  if (
    !isRecord(value) ||
    typeof value.code !== "string" ||
    value.code.length === 0 ||
    typeof value.expires_at !== "string" ||
    value.expires_at.length === 0 ||
    (value.deep_link !== undefined &&
      value.deep_link !== null &&
      typeof value.deep_link !== "string")
  ) {
    throw new TypeError(`invalid ${responseName} response`);
  }
  const deepLinkValue = value.deep_link;
  const deepLink =
    typeof deepLinkValue === "string" ? deepLinkValue : undefined;
  return {
    code: value.code,
    ...(deepLink === undefined ? {} : { deep_link: deepLink }),
    expires_at: value.expires_at,
  };
}

function decodePairingStatus(value: unknown): ExtensionPairingStatus {
  if (!isRecord(value) || typeof value.connected !== "boolean") {
    throw new TypeError("invalid pairing status response");
  }
  return {
    connected: value.connected,
    pending:
      value.pending === undefined || value.pending === null
        ? null
        : decodePairingCode(value.pending, "pairing status"),
  };
}

// -> { code, deep_link?, expires_at }; mints (or rotates) the caller's code.
export async function mintExtensionPairingCode(
  extensionId: string,
): Promise<ExtensionPairingCode> {
  const response = await apiFetch(extensionPairingPath(extensionId, "mint"), {
    method: "POST",
  });
  return decodePairingCode(response);
}

// -> { connected, pending: { code, deep_link?, expires_at } | null }
export async function getExtensionPairingStatus(
  extensionId: string,
): Promise<ExtensionPairingStatus> {
  const response = await apiFetch(extensionPairingPath(extensionId, "status"));
  return decodePairingStatus(response);
}

// -> 204; unpairs the caller's account on this channel.
export async function unpairExtension(extensionId: string): Promise<void> {
  await apiFetch(extensionPairingPath(extensionId, "unpair"), { method: "POST" });
}

export const extensionPairingError = channelSetupError;
