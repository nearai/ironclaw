export const CONNECTION_STATUS = Object.freeze({
  IDLE: "idle",
  CONNECTING: "connecting",
  CONNECTED: "connected",
  RECONNECTING: "reconnecting",
  DISCONNECTED: "disconnected",
  PAUSED: "paused",
});

const CONNECTION_LOST_STATUSES = new Set([
  CONNECTION_STATUS.DISCONNECTED,
  CONNECTION_STATUS.RECONNECTING,
]);

function normalizeConnectionStatus(status) {
  return typeof status === "string" ? status.trim().toLowerCase() : "";
}

export function isConnectionLostStatus(status) {
  return CONNECTION_LOST_STATUSES.has(normalizeConnectionStatus(status));
}
