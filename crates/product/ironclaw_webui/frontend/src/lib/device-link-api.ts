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
// TODO(design): these paths are the browser half of the PR 4 backend route
// ("route input submission to the driver", PROPOSAL §8.12), which is not on
// this branch yet. The shapes mirror `DeviceLinkFlowDriver`'s
// `start`/`poll`/`submit_input`/`cancel` and the `OAuth*` route naming already
// in `crates/product/ironclaw_webui/src/product_auth/mod.rs`; if the landed
// route names differ, this module is the single place to reconcile them.
const DEVICE_LINK_BASE = "/api/reborn/product-auth/device-link";

export function deviceLinkFlowPath(flowId, action) {
  return `${DEVICE_LINK_BASE}/flow/${encodeURIComponent(flowId)}/${action}`;
}

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
  return apiFetch(`${DEVICE_LINK_BASE}/start`, {
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
  return apiFetch(`${deviceLinkFlowPath(flowId, "status")}${query}`, { signal });
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
  return apiFetch(deviceLinkFlowPath(flowId, "input"), {
    method: "POST",
    signal,
    body: JSON.stringify({
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
  return apiFetch(deviceLinkFlowPath(flowId, "cancel"), {
    method: "POST",
    signal,
    body: JSON.stringify({ invocation_id: invocationId }),
  });
}

export const deviceLinkError = channelSetupError;
