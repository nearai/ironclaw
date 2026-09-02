import { apiFetch, type ApiRecord } from "./api";

const V2_BASE = "/api/webchat/v2";

export interface NotificationSetupOptions {
  extensionId?: string;
  payload?: unknown;
}

export interface NotificationSetupStatusResponse extends ApiRecord {
  extension_id: string;
  requires_setup: boolean;
  enabled: boolean;
  detail?: unknown;
}

function decodeNotificationSetupStatus(
  value: ApiRecord,
): NotificationSetupStatusResponse {
  if (
    typeof value.extension_id !== "string" ||
    !value.extension_id ||
    typeof value.requires_setup !== "boolean" ||
    typeof value.enabled !== "boolean"
  ) {
    throw new TypeError("invalid notification setup response");
  }
  return value as NotificationSetupStatusResponse;
}

function notificationSetupPath(extensionId: string, action = ""): string {
  const suffix = action ? `/${action}` : "";
  return `${V2_BASE}/channels/${encodeURIComponent(extensionId)}/notifications${suffix}`;
}

export async function getNotificationSetupStatus({
  extensionId,
}: NotificationSetupOptions = {}): Promise<NotificationSetupStatusResponse> {
  if (!extensionId) throw new Error("extensionId is required");
  return decodeNotificationSetupStatus(
    await apiFetch(notificationSetupPath(extensionId), { cache: "no-store" }),
  );
}

export async function enableNotificationSetup({
  extensionId,
  payload,
}: NotificationSetupOptions = {}): Promise<NotificationSetupStatusResponse> {
  if (!extensionId) throw new Error("extensionId is required");
  return decodeNotificationSetupStatus(
    await apiFetch(notificationSetupPath(extensionId, "enable"), {
      method: "POST",
      body: JSON.stringify({ payload }),
    }),
  );
}

export async function disableNotificationSetup({
  extensionId,
  payload,
}: NotificationSetupOptions = {}): Promise<NotificationSetupStatusResponse> {
  if (!extensionId) throw new Error("extensionId is required");
  return decodeNotificationSetupStatus(
    await apiFetch(notificationSetupPath(extensionId, "disable"), {
      method: "POST",
      body: JSON.stringify({ payload }),
    }),
  );
}
