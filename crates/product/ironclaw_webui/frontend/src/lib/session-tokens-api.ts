// @ts-nocheck
// DEMO SCOPE: self-serve bearer mint for a client that cannot complete the
// browser session flow itself. Superseded by device-code pairing; delete
// with the Settings Devices tab.
import { V2_BASE, apiFetch } from "./api";

// -> { token }; mints a new bearer for the caller.
export function mintSessionToken() {
  return apiFetch(`${V2_BASE}/session/tokens`, { method: "POST" });
}
