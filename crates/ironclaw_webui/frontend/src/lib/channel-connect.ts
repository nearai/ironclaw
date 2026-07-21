import { apiFetch } from "./api";

export function listConnectableChannels() {
  return apiFetch("/api/webchat/v2/channels/connectable");
}
