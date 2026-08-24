// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  DEVICE_LINK_DEFAULT_POLL_MS,
  DEVICE_LINK_DISPLAY_KINDS,
  DEVICE_LINK_INPUT_KINDS,
  DEVICE_LINK_MODES,
  DEVICE_LINK_STEPS,
  deviceLinkAlternateMode,
  deviceLinkFrameFromWire,
  deviceLinkModeLabel,
  deviceLinkPollDelayMs,
  deviceLinkStepIsTerminal,
} from "./device-link-frame";

const EXPIRES_AT = "2026-07-16T12:01:30Z";

test("deviceLinkFrameFromWire normalizes a full frame to camelCase", () => {
  const frame = deviceLinkFrameFromWire({
    flow_id: "flow-1",
    provider: "example",
    extension_id: "example_chat",
    display_name: "Example",
    step: DEVICE_LINK_STEPS.inputRequired,
    instructions: "Enter the code we sent you.",
    qr_payload: "scheme://login?token=AAAA",
    display_kind: DEVICE_LINK_DISPLAY_KINDS.qrCode,
    code: "AB-CD-12",
    vendor_user_ref: "@person",
    secret_label: "Login code",
    input_kind: DEVICE_LINK_INPUT_KINDS.password,
    mode: DEVICE_LINK_MODES.alternate,
    alternate_available: true,
    default_mode_label: "Scan a code",
    alternate_mode_label: "Use my phone number",
    expires_at: EXPIRES_AT,
    revision: 4,
    poll_interval_ms: 3000,
    retry_after_ms: 30_000,
    error_code: "rate_limited",
    restartable: false,
  });

  assert.deepEqual(frame, {
    flowId: "flow-1",
    provider: "example",
    // The credential authority and the installed extension are separate
    // identities and neither substitutes for the other.
    extensionId: "example_chat",
    displayName: "Example",
    step: DEVICE_LINK_STEPS.inputRequired,
    instructions: "Enter the code we sent you.",
    qrPayload: "scheme://login?token=AAAA",
    displayKind: DEVICE_LINK_DISPLAY_KINDS.qrCode,
    // `code` is a short code the vendor issued for the user to READ; the
    // resolved account identity rides its own slot.
    code: "AB-CD-12",
    vendorUserRef: "@person",
    secretLabel: "Login code",
    inputKind: DEVICE_LINK_INPUT_KINDS.password,
    mode: DEVICE_LINK_MODES.alternate,
    alternateAvailable: true,
    defaultModeLabel: "Scan a code",
    alternateModeLabel: "Use my phone number",
    expiresAtMs: Date.parse(EXPIRES_AT),
    revision: 4,
    pollIntervalMs: 3000,
    retryAfterMs: 30_000,
    errorCode: "rate_limited",
    restartable: false,
    terminal: false,
  });
});

test("deviceLinkFrameFromWire rejects anything that is not a frame", () => {
  assert.equal(deviceLinkFrameFromWire(null), null);
  assert.equal(deviceLinkFrameFromWire(undefined), null);
  assert.equal(deviceLinkFrameFromWire("display"), null);
  assert.equal(deviceLinkFrameFromWire({}), null, "a frame with no step is not renderable");
});

test("deviceLinkFrameFromWire fills the fields the prompt view does not carry", () => {
  // `DeviceLinkPromptView` carries no input kind, mode, or restartable flag —
  // those are the additive flow-status fields. The card still has to render, so
  // each falls back deterministically.
  const frame = deviceLinkFrameFromWire({
    provider: "example",
    display_name: "Example",
    step: DEVICE_LINK_STEPS.display,
    instructions: "Scan this.",
    expires_at: EXPIRES_AT,
    revision: 1,
  });

  assert.equal(frame.inputKind, DEVICE_LINK_INPUT_KINDS.code);
  assert.equal(frame.mode, DEVICE_LINK_MODES.default);
  assert.equal(frame.pollIntervalMs, DEVICE_LINK_DEFAULT_POLL_MS);
  assert.equal(frame.retryAfterMs, 0);
  assert.equal(frame.flowId, null);
  assert.equal(frame.errorCode, null);

  // The recipe-shaped fields fall back to "the extension said nothing", which
  // is what makes the card's ceremony come off the wire rather than out of one
  // vendor's habits.
  assert.equal(
    frame.alternateAvailable,
    false,
    "an unstated second path must be assumed absent: offering a switch a vendor rejects wedges the user",
  );
  assert.equal(frame.defaultModeLabel, null);
  assert.equal(frame.alternateModeLabel, null);
  assert.equal(
    frame.displayKind,
    null,
    "an unstated display kind leaves the card rendering both affordances",
  );
  assert.equal(frame.vendorUserRef, null);
  assert.equal(frame.extensionId, null);
});

test("an unrecognized display kind normalizes to unstated, not to nothing renderable", () => {
  // A newer host naming a kind this browser has never heard of. Matching on
  // the raw string would render NEITHER affordance — a display step with
  // nothing on it — so it degrades to "both", the pre-field behavior.
  const frame = deviceLinkFrameFromWire({
    provider: "example",
    display_name: "Example",
    step: DEVICE_LINK_STEPS.display,
    instructions: "Scan this.",
    qr_payload: "scheme://login?token=AAAA",
    display_kind: "holographic_projection",
    expires_at: EXPIRES_AT,
    revision: 1,
  });

  assert.equal(frame.displayKind, null);
});

test("the mode switch takes its label from the recipe, for whichever path it moves to", () => {
  const frame = deviceLinkFrameFromWire({
    provider: "example",
    display_name: "Example",
    step: DEVICE_LINK_STEPS.display,
    instructions: "Scan this.",
    expires_at: EXPIRES_AT,
    revision: 1,
    alternate_available: true,
    default_mode_label: "Scan a code",
    alternate_mode_label: "Use my phone number",
  });

  assert.equal(deviceLinkModeLabel(frame, DEVICE_LINK_MODES.default), "Scan a code");
  assert.equal(deviceLinkModeLabel(frame, DEVICE_LINK_MODES.alternate), "Use my phone number");

  // No label is the signal to fall back to generic host copy — never to one
  // vendor's ceremony.
  const unlabelled = deviceLinkFrameFromWire({
    provider: "example",
    display_name: "Example",
    step: DEVICE_LINK_STEPS.display,
    instructions: "Scan this.",
    expires_at: EXPIRES_AT,
    revision: 1,
    alternate_available: true,
  });
  assert.equal(deviceLinkModeLabel(unlabelled, DEVICE_LINK_MODES.default), null);
  assert.equal(deviceLinkModeLabel(unlabelled, DEVICE_LINK_MODES.alternate), null);
  assert.equal(deviceLinkModeLabel(null, DEVICE_LINK_MODES.alternate), null);
});

test("restartability falls back to the driver's own rule when the frame omits it", () => {
  const failedWith = (code) =>
    deviceLinkFrameFromWire({
      provider: "example",
      display_name: "Example",
      step: DEVICE_LINK_STEPS.failed,
      instructions: "Linking did not finish.",
      expires_at: EXPIRES_AT,
      revision: 2,
      error_code: code,
    }).restartable;

  // Mirrors `DeviceLinkDriverError::restartable`: an ineligible account and a
  // custody failure cannot be fixed by starting over.
  assert.equal(failedWith("account_unavailable"), false);
  assert.equal(failedWith("custody_failed"), false);
  assert.equal(failedWith("expired"), true);
  assert.equal(failedWith("unknown_flow"), true);
  assert.equal(failedWith("rate_limited"), true);
  assert.equal(failedWith("declined"), true);

  // An explicit flag always wins over the fallback.
  assert.equal(
    deviceLinkFrameFromWire({
      provider: "example",
      display_name: "Example",
      step: DEVICE_LINK_STEPS.failed,
      instructions: "Linking did not finish.",
      expires_at: EXPIRES_AT,
      revision: 2,
      error_code: "expired",
      restartable: false,
    }).restartable,
    false,
  );
});

test("only completed and failed are terminal", () => {
  assert.equal(deviceLinkStepIsTerminal(DEVICE_LINK_STEPS.completed), true);
  assert.equal(deviceLinkStepIsTerminal(DEVICE_LINK_STEPS.failed), true);
  assert.equal(deviceLinkStepIsTerminal(DEVICE_LINK_STEPS.display), false);
  assert.equal(deviceLinkStepIsTerminal(DEVICE_LINK_STEPS.awaitingVendor), false);
  assert.equal(deviceLinkStepIsTerminal(DEVICE_LINK_STEPS.inputRequired), false);
});

test("a vendor back-off overrides the frame's own poll pace, for one poll", () => {
  assert.equal(deviceLinkPollDelayMs({ pollIntervalMs: 3000, retryAfterMs: 0 }), 3000);
  assert.equal(deviceLinkPollDelayMs({ pollIntervalMs: 3000, retryAfterMs: 30_000 }), 30_000);
  assert.equal(deviceLinkPollDelayMs(null), DEVICE_LINK_DEFAULT_POLL_MS);
});

test("the alternate mode toggles both ways", () => {
  assert.equal(deviceLinkAlternateMode(DEVICE_LINK_MODES.default), DEVICE_LINK_MODES.alternate);
  assert.equal(deviceLinkAlternateMode(DEVICE_LINK_MODES.alternate), DEVICE_LINK_MODES.default);
  assert.equal(deviceLinkAlternateMode(undefined), DEVICE_LINK_MODES.alternate);
});
