// @ts-nocheck
// One normalizer for the device-link frame, shared by every surface that
// renders one.
//
// Source shape: `DeviceLinkPromptView`
// (`crates/contracts/ironclaw_extension_contracts/src/auth_prompt.rs`), which
// rides the chat gate as `prompt.device_link` / `gate.auth_context.device_link`
// and the flow-status route as `response.device_link`. Both arrive here so a
// polled frame and a gate frame can never be normalized two different ways.
//
// Everything in the frame is presentation. The step machine, its revision
// compare-and-swap, and the TTLs belong to the auth engine; the browser only
// paints the current frame, echoes `revision` back on submit, and obeys the
// poll pacing it is handed.

// Vendor-issued payload rendering (`DeviceLinkStepKind`).
export const DEVICE_LINK_STEPS = Object.freeze({
  display: "display",
  awaitingVendor: "awaiting_vendor",
  inputRequired: "input_required",
  completed: "completed",
  failed: "failed",
});

// `DeviceLinkInputKind` — what the current step is asking the user for.
export const DEVICE_LINK_INPUT_KINDS = Object.freeze({
  identifier: "identifier",
  code: "code",
  password: "password",
});

// `DeviceLinkMode` — the vendor's primary path vs. its declared fallback
// (e.g. scan a QR on the default path, type a phone number on the
// alternate one).
export const DEVICE_LINK_MODES = Object.freeze({
  default: "default",
  alternate: "alternate",
});

// The pace a card polls at when the frame declares none.
export const DEVICE_LINK_DEFAULT_POLL_MS = 3000;

// A step that will never advance again: nothing left to poll for. A card left
// open on one of these must hold no timer.
const TERMINAL_STEPS = Object.freeze([DEVICE_LINK_STEPS.completed, DEVICE_LINK_STEPS.failed]);

// `DeviceLinkErrorCode` — the closed failure vocabulary the host publishes. A
// code outside this list is a newer host talking to an older browser: render
// the frame's own copy rather than a missing translation key.
export const DEVICE_LINK_ERROR_CODES = Object.freeze([
  "expired",
  "unknown_flow",
  "declined",
  "invalid_input",
  "rate_limited",
  "account_unavailable",
  "vendor_unavailable",
  "custody_failed",
  "internal",
]);

// Failures that a fresh `begin` cannot fix, mirroring
// `DeviceLinkDriverError::restartable` in `ironclaw_auth`: the account is
// ineligible, or host-side custody failed. Every other code is worth another
// attempt. Used only as the fallback when the frame does not state it.
const NON_RESTARTABLE_ERROR_CODES = Object.freeze(["account_unavailable", "custody_failed"]);

export function deviceLinkStepIsTerminal(step) {
  return TERMINAL_STEPS.includes(step);
}

function optionalText(value) {
  return typeof value === "string" && value ? value : null;
}

function boundedNumber(value) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
}

// TODO(design): `input_kind`, `mode`, `restartable`, and `flow_id` are the
// additive flow-status fields PROPOSAL §8.12 assigns to the WebUI backend half
// of PR 4; `DeviceLinkPromptView` itself carries none of them. Until that route
// lands, each is read when present and falls back below:
//   - `input_kind` -> "code" (the only input every device link asks for; the
//     masked-password affordance therefore needs the real field),
//   - `mode` -> "default",
//   - `restartable` -> derived from `error_code`,
//   - `flow_id` -> absent, and a card with no flow id cannot poll or submit,
//     so it starts a flow of its own instead.
export function deviceLinkFrameFromWire(wire) {
  if (!wire || typeof wire !== "object") return null;
  const step = optionalText(wire.step);
  if (!step) return null;
  const expiresAt = Date.parse(wire.expires_at || "");
  const errorCode = optionalText(wire.error_code);
  return {
    flowId: optionalText(wire.flow_id),
    provider: String(wire.provider || ""),
    displayName: String(wire.display_name || ""),
    step,
    instructions: String(wire.instructions || ""),
    qrPayload: optionalText(wire.qr_payload),
    code: optionalText(wire.code),
    secretLabel: optionalText(wire.secret_label),
    inputKind: optionalText(wire.input_kind) || DEVICE_LINK_INPUT_KINDS.code,
    mode: optionalText(wire.mode) || DEVICE_LINK_MODES.default,
    expiresAtMs: Number.isFinite(expiresAt) ? expiresAt : 0,
    revision: boundedNumber(wire.revision),
    pollIntervalMs: boundedNumber(wire.poll_interval_ms) || DEVICE_LINK_DEFAULT_POLL_MS,
    // A vendor back-off overrides the pace for the next poll only, so it stays
    // a distinct field rather than being folded into `pollIntervalMs`.
    retryAfterMs: boundedNumber(wire.retry_after_ms) || 0,
    errorCode,
    restartable:
      typeof wire.restartable === "boolean"
        ? wire.restartable
        : !NON_RESTARTABLE_ERROR_CODES.includes(errorCode),
    terminal: deviceLinkStepIsTerminal(step),
  };
}

// The delay before the next poll: a vendor-requested back-off wins, otherwise
// the frame's own pace.
export function deviceLinkPollDelayMs(frame) {
  if (!frame) return DEVICE_LINK_DEFAULT_POLL_MS;
  return frame.retryAfterMs || frame.pollIntervalMs || DEVICE_LINK_DEFAULT_POLL_MS;
}

// The mode a "use the other path instead" affordance switches to.
export function deviceLinkAlternateMode(mode) {
  return mode === DEVICE_LINK_MODES.alternate
    ? DEVICE_LINK_MODES.default
    : DEVICE_LINK_MODES.alternate;
}
