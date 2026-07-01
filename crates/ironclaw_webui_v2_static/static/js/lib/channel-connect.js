import { apiFetch } from "./api.js";

export function listConnectableChannels() {
  return apiFetch("/api/webchat/v2/channels/connectable");
}
