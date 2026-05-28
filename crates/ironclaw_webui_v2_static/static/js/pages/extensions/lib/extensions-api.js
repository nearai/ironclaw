// Extensions surface:
// - `submitExtensionSetup` is the one function that wires to a real v2
//   endpoint (`POST /api/webchat/v2/extensions/{name}/setup`).
//   The v2 facade currently returns `status: not_implemented`, which is
//   surfaced as-is to the caller — see issue #3886 implementation notes
//   and `ironclaw_webui_v2/CLAUDE.md` "Setup-extension (skeleton)".
// - Every other function is a TODO stub returning empty data so the
//   fork's extensions UI renders without hitting any v1 path.

import { setupExtension } from "../../../lib/api.js";

export function fetchExtensions() {
  return Promise.resolve({ extensions: [], todo: true });
}
export function fetchExtensionRegistry() {
  return Promise.resolve({ entries: [], todo: true });
}
export function installExtension(_name, _kind) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 install endpoint" });
}
export function activateExtension(_name) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 activate endpoint" });
}
export function removeExtension(_name) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 remove endpoint" });
}
export function fetchExtensionSetup(_name) {
  // v2 has no GET counterpart for setup — the configure modal then
  // renders its "no configuration required" empty branch and submit
  // becomes a no-op POST that returns `not_implemented`.
  return Promise.resolve({
    secrets: [],
    fields: [],
    onboarding: null,
    todo: true,
  });
}
export function submitExtensionSetup(name, secrets, fields) {
  return setupExtension(name, {
    action: "submit",
    payload: { secrets, fields },
  });
}
export function fetchPairingRequests(_channel) {
  return Promise.resolve({ requests: [], todo: true });
}
export function approvePairingCode(_channel, _code) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 pairing endpoint" });
}
