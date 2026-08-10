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
// These call the EXISTING generic product-auth routes. A device link is an
// `AuthFlowRecord` like any other, and `flow_status(scope, flow_id)` is generic
// over flows — the route is merely *named* `oauth/...` for historical reasons.
// PROPOSAL §8.12 is explicit that the backend work is "additive flow-status
// fields (step, revision, display, retry-after); route input submission to the
// driver" — i.e. EXTEND these, never a parallel `/device-link` namespace.
//
// TODO(backend): the status route must carry the additive device-link frame,
// and secret submission must route to the device-link driver. Both are small
// extensions of the handlers already mounted in
// `crates/product/ironclaw_webui/src/product_auth/mod.rs`.
// The generic flow-status route (shared with OAuth — same record type).
export function deviceLinkStatusPath(flowId) {
  return `/api/reborn/product-auth/oauth/flow/${encodeURIComponent(flowId)}/status`;
}

// Start rides the extension auth-start route; input rides the generic
// secret-submit route. Neither is device-link specific.
const START_PATH = "/api/reborn/product-auth/extension/oauth/start";
const INPUT_PATH = "/api/reborn/product-auth/manual-token/secret/submit";

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
  return apiFetch(`/api/reborn/product-auth/oauth/flow/${encodeURIComponent(flowId)}/reconcile`, {
    method: "POST",
    signal,
    body: JSON.stringify({ invocation_id: invocationId, cancel: true }),
  });
}

export const deviceLinkError = channelSetupError;
