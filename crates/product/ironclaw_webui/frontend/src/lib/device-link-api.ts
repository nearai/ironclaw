// @ts-nocheck
import { apiFetch } from "./api";
import { channelSetupError } from "./channel-setup-api";

// Device-link flow routes (PROPOSAL §4, §8.12). They sit next to the OAuth
// flow routes because a device link is the same kind of object: a durable,
// caller-scoped `AuthFlowRecord` the browser advances and polls. What differs
// is that an OAuth flow has one transition and a device link has a sequence of
// them, so every call here carries the flow id and every submission echoes the
// revision it was rendered from.
//
// Nothing secret is stored by this module. Codes and passwords are handed
// straight to `submitDeviceLinkInput` and never retained: the host takes
// custody of the resulting session, and the browser never sees it.
//
// Route shape, and why it is a mix.
//
// STATUS is genuinely shared: a device link IS an `AuthFlowRecord`, and
// `flow_status(scope, flow_id)` fetches the record and returns its status with
// no OAuth-specific logic. PROPOSAL §8.12 asks for "additive flow-status
// fields" on exactly this response — so polling extends the existing route.
// NAMING WART: that route is spelled `oauth/flow/...` for historical reasons
// even though the object it serves is generic. Renaming it to
// `/product-auth/flow/{flow_id}/status` (keeping the old spelling as an alias
// for shipped OAuth clients) is the right follow-up; it is a route-descriptor
// change, not part of this feature.
//
// START, INPUT and CANCEL are NOT shared, because the operations differ:
//   - start takes a link MODE (QR vs phone); OAuth start builds an authorize URL.
//   - input carries a typed kind (identifier | code | password) plus the step
//     REVISION it was typed against; manual-token submit is "paste an API key".
//   - cancel must ask the vendor to log the device out, or an accepted-but-
//     abandoned link leaves an orphan authorization on the user's account
//     (PROPOSAL §4.3). Nothing existing does that.
//
// TODO(backend): the three device-link routes below must be added to
// `crates/product/ironclaw_webui/src/product_auth/mod.rs` and dispatched to
// `DeviceLinkFlowDriver`. They are work this feature owes, NOT a dependency on
// another branch — an earlier revision of this file claimed the latter.
// The generic flow-status route (shared with OAuth — same record type).
export function deviceLinkStatusPath(flowId) {
  return `/api/reborn/product-auth/oauth/flow/${encodeURIComponent(flowId)}/status`;
}

// Device-link specific — see the header. Not yet mounted; TODO(backend).
const DEVICE_LINK_BASE = "/api/reborn/product-auth/device-link";
const START_PATH = `${DEVICE_LINK_BASE}/start`;
const INPUT_PATH = `${DEVICE_LINK_BASE}/input`;
const CANCEL_PATH = `${DEVICE_LINK_BASE}/cancel`;

// -> { flow_id, status, device_link }
//
// `resumeFlowId` is what makes a re-rendered card (a refresh, a second tab, a
// re-opened settings pane) resume the live link instead of burning the payload
// the user is mid-scan on. A stale or lapsed id falls through to a fresh link
// server-side rather than failing.
export function startDeviceLink({
  provider,
  extensionName,
  mode,
  threadId,
  runId,
  gateRef,
  invocationId,
  resumeFlowId,
  signal,
} = {}) {
  return apiFetch(START_PATH, {
    method: "POST",
    signal,
    body: JSON.stringify({
      provider,
      extension_name: extensionName,
      mode,
      thread_id: threadId,
      run_id: runId,
      gate_ref: gateRef,
      invocation_id: invocationId,
      resume_flow_id: resumeFlowId,
    }),
  });
}

// -> { flow_id, status, device_link }; a pure read, safe to call while the
// card is awaiting user input.
export function pollDeviceLink({ flowId, invocationId, signal } = {}) {
  const query = invocationId
    ? `?invocation_id=${encodeURIComponent(invocationId)}`
    : "";
  return apiFetch(`${deviceLinkStatusPath(flowId)}${query}`, { signal });
}

// -> { flow_id, status, device_link }
//
// `revision` is the frame the value was typed against. The engine's
// compare-and-swap rejects a submission from a superseded frame, which is what
// stops a stale card from overwriting newer state — never drop it.
export function submitDeviceLinkInput({
  flowId,
  revision,
  kind,
  value,
  invocationId,
  signal,
} = {}) {
  return apiFetch(INPUT_PATH, {
    method: "POST",
    signal,
    body: JSON.stringify({
      flow_id: flowId,
      revision,
      kind,
      value,
      invocation_id: invocationId,
    }),
  });
}

// -> { flow_id, status }; abandons the flow so the vendor side is logged out
// rather than left as an orphan authorization (PROPOSAL §4.3).
export function cancelDeviceLink({ flowId, invocationId, signal } = {}) {
  return apiFetch(CANCEL_PATH, {
    method: "POST",
    signal,
    body: JSON.stringify({ flow_id: flowId, invocation_id: invocationId }),
  });
}

export const deviceLinkError = channelSetupError;
