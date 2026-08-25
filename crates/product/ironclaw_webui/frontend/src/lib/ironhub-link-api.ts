// @ts-nocheck
import { apiFetch } from "./api";

// IronHub mints the shared key per agent installation; the agent only accepts
// it. Every route here is operator-gated and answers 403 for a non-admin caller.
const IRONHUB_LINK_PATH = "/api/webchat/v2/ironhub/link";

// -> { register_url, key_stored, key_active }; never key material.
export function getIronhubLink() {
  return apiFetch(IRONHUB_LINK_PATH);
}

// -> the same shape, re-read after the key lands.
export function setIronhubSharedKey(sharedKey) {
  return apiFetch(`${IRONHUB_LINK_PATH}/key`, {
    method: "POST",
    body: JSON.stringify({ shared_key: sharedKey }),
  });
}

// -> the same shape. A running gateway keeps the key it booted with, so
// key_active can stay true until the next restart.
export function clearIronhubSharedKey() {
  return apiFetch(`${IRONHUB_LINK_PATH}/key`, { method: "DELETE" });
}
