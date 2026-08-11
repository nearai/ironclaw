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
// STATUS is genuinely shared and is a pure READ: a device link IS an
// `AuthFlowRecord`, and the generic flow-status route fetches that record with
// no OAuth-specific logic. PROPOSAL §8.12's additive fields ride exactly this
// response, so a card that re-renders (a refresh, a second tab, a re-opened
// settings pane) hydrates from it without disturbing the live link.
// NAMING WART: that route is spelled `oauth/flow/...` for historical reasons
// even though the object it serves is generic. Renaming it to
// `/product-auth/flow/{flow_id}/status` (keeping the old spelling as an alias
// for shipped OAuth clients) is the right follow-up; it is a route-descriptor
// change, not part of this feature.
//
// START, POLL, INPUT and CANCEL are NOT shared, because the operations differ
// — every one of them makes a vendor-visible transition:
//   - start takes a link MODE (QR vs phone); OAuth start builds an authorize URL.
//   - poll ASKS THE VENDOR whether the code was accepted. An earlier revision of
//     this file had the card poll the read-only status route instead; that
//     cannot work. A device link only advances when the host re-exports the
//     login token (PROPOSAL §4.2 — acceptance is poll-driven), nothing else
//     drives it, and a card polling a pure read waits forever on a QR that was
//     already scanned. The host's own poll floor keeps this cheap: a too-early
//     poll is answered without the vendor being called at all.
//   - input carries a typed kind (identifier | code | password) plus the step
//     REVISION it was typed against; manual-token submit is "paste an API key".
//   - cancel must ask the vendor to log the device out, or an accepted-but-
//     abandoned link leaves an orphan authorization on the user's account
//     (PROPOSAL §4.3). Nothing existing does that.
// The generic flow-status route (shared with OAuth — same record type).
export function deviceLinkStatusPath(flowId) {
  return `/api/reborn/product-auth/oauth/flow/${encodeURIComponent(flowId)}/status`;
}

// Drop optional identifiers the caller does not have.
//
// An absent id must be OMITTED, never sent as `""`. The host parses every one
// of these into a validated newtype (`ThreadId`, `InvocationId`, `TurnRunRef`,
// `AuthGateRef`), and a blank string fails that parse — so a body carrying
// `thread_id: ""` is rejected with `invalid_request` before the flow is ever
// started. The gate model has no `threadId` at all and leaves `invocationId`
// null, so a card that defaulted those to `""` could never start a link.
//
// Required fields (`provider`, `extension_name`) are deliberately NOT filtered:
// blank ones must reach the host and be rejected, not silently vanish into a
// request that means something else.
function withoutBlankIds(body) {
  const kept = {};
  for (const [key, value] of Object.entries(body)) {
    if (value === undefined || value === null || value === "") continue;
    kept[key] = value;
  }
  return kept;
}

// Device-link specific — see the header.
const DEVICE_LINK_BASE = "/api/reborn/product-auth/device-link";
const START_PATH = `${DEVICE_LINK_BASE}/start`;
const POLL_PATH = `${DEVICE_LINK_BASE}/poll`;
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
      ...withoutBlankIds({
        mode,
        thread_id: threadId,
        run_id: runId,
        gate_ref: gateRef,
        invocation_id: invocationId,
        resume_flow_id: resumeFlowId,
      }),
    }),
  });
}

// -> { flow_id, status, device_link }
//
// Advances the link: the host asks the vendor whether the displayed code was
// accepted. Safe to call while the card is awaiting user input — the adapter's
// poll is contractually a pure read on that side, and the host serializes it
// against a submission in flight.
export function pollDeviceLink({ flowId, invocationId, signal } = {}) {
  return apiFetch(POLL_PATH, {
    method: "POST",
    signal,
    body: JSON.stringify({
      flow_id: flowId,
      ...withoutBlankIds({ invocation_id: invocationId }),
    }),
  });
}

// -> { flow_id, status, device_link }; a pure READ of the durable flow, for a
// card that is re-rendering and wants to hydrate without advancing anything.
export function readDeviceLinkFlow({ flowId, invocationId, signal } = {}) {
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
      ...withoutBlankIds({ invocation_id: invocationId }),
    }),
  });
}

// -> { flow_id, status }; abandons the flow so the vendor side is logged out
// rather than left as an orphan authorization (PROPOSAL §4.3).
export function cancelDeviceLink({ flowId, invocationId, signal } = {}) {
  return apiFetch(CANCEL_PATH, {
    method: "POST",
    signal,
    body: JSON.stringify({
      flow_id: flowId,
      ...withoutBlankIds({ invocation_id: invocationId }),
    }),
  });
}

export const deviceLinkError = channelSetupError;
