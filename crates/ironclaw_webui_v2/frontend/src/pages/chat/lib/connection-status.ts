export const CONNECTION_STATUS = Object.freeze({
  IDLE: "idle",
  CONNECTING: "connecting",
  CONNECTED: "connected",
  RECONNECTING: "reconnecting",
  DISCONNECTED: "disconnected",
  PAUSED: "paused",
} as const);

export type ConnectionStatus =
  (typeof CONNECTION_STATUS)[keyof typeof CONNECTION_STATUS];

const CONNECTION_LOST_STATUSES: ReadonlySet<string> = new Set([
  CONNECTION_STATUS.DISCONNECTED,
  CONNECTION_STATUS.RECONNECTING,
]);

function normalizeConnectionStatus(status: unknown): string {
  return typeof status === "string" ? status.trim().toLowerCase() : "";
}

export function isConnectionLostStatus(status: unknown): boolean {
  return CONNECTION_LOST_STATUSES.has(normalizeConnectionStatus(status));
}
