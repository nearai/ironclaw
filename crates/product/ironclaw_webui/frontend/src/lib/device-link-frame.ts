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

// `DeviceLinkDisplayKind` — what the payload on a display step IS, and so which
// affordance renders it: a scannable code, or a link the user opens on the
// device being linked. A frame that declares none renders both, exactly as
// every frame did before the field existed.
export const DEVICE_LINK_DISPLAY_KINDS = Object.freeze({
  qrCode: "qr_code",
  link: "link",
});

export type DeviceLinkStep = (typeof DEVICE_LINK_STEPS)[keyof typeof DEVICE_LINK_STEPS];
export type DeviceLinkInputKind =
  (typeof DEVICE_LINK_INPUT_KINDS)[keyof typeof DEVICE_LINK_INPUT_KINDS];
export type DeviceLinkMode = (typeof DEVICE_LINK_MODES)[keyof typeof DEVICE_LINK_MODES];
export type DeviceLinkDisplayKind =
  (typeof DEVICE_LINK_DISPLAY_KINDS)[keyof typeof DEVICE_LINK_DISPLAY_KINDS];

export interface DeviceLinkPromptWire {
  provider: string;
  display_name: string;
  step: DeviceLinkStep;
  instructions: string;
  expires_at: string;
  revision: number;
  poll_interval_ms: number;
  qr_payload?: string | null;
  code?: string | null;
  vendor_user_ref?: string | null;
  secret_label?: string | null;
  retry_after_ms?: number | null;
  error_code?: string | null;
  flow_id?: string | null;
  input_kind?: DeviceLinkInputKind | null;
  mode?: DeviceLinkMode | null;
  alternate_available?: boolean | null;
  default_mode_label?: string | null;
  alternate_mode_label?: string | null;
  display_kind?: DeviceLinkDisplayKind | null;
  extension_id?: string | null;
  restartable?: boolean | null;
}

export interface DeviceLinkFrame {
  flowId: string | null;
  provider: string;
  extensionId: string | null;
  displayName: string;
  step: string;
  instructions: string;
  qrPayload: string | null;
  displayKind: DeviceLinkDisplayKind | null;
  code: string | null;
  vendorUserRef: string | null;
  secretLabel: string | null;
  inputKind: string;
  mode: string;
  alternateAvailable: boolean;
  defaultModeLabel: string | null;
  alternateModeLabel: string | null;
  expiresAtMs: number;
  revision: number;
  pollIntervalMs: number;
  retryAfterMs: number;
  errorCode: string | null;
  restartable: boolean;
  terminal: boolean;
}

// The pace a card polls at when the frame declares none.
export const DEVICE_LINK_DEFAULT_POLL_MS = 3000;

// A step that will never advance again: nothing left to poll for. A card left
// open on one of these must hold no timer.
const TERMINAL_STEPS: readonly string[] = Object.freeze([
  DEVICE_LINK_STEPS.completed,
  DEVICE_LINK_STEPS.failed,
]);

// `DeviceLinkErrorCode` — the closed failure vocabulary the host publishes. A
// code outside this list is a newer host talking to an older browser: render
// the frame's own copy rather than a missing translation key.
export const DEVICE_LINK_ERROR_CODES = Object.freeze([
  "expired",
  "unknown_flow",
  "declined",
  "invalid_input",
  "rate_limited",
  "host_throttled",
  "limit_reached",
  "account_unavailable",
  "identity_conflict",
  "vendor_unavailable",
  "custody_failed",
  "internal",
]);

// Failures that a fresh `begin` cannot fix, mirroring
// `DeviceLinkDriverError::restartable` in `ironclaw_auth`: the account is
// ineligible, or host-side custody failed. Every other code is worth another
// attempt. Used only as the fallback when the frame does not state it.
const NON_RESTARTABLE_ERROR_CODES: readonly string[] = Object.freeze([
  "account_unavailable",
  "identity_conflict",
  "custody_failed",
]);

export function deviceLinkStepIsTerminal(step: unknown): boolean {
  return typeof step === "string" && TERMINAL_STEPS.includes(step);
}

function optionalText(value: unknown): string | null {
  return typeof value === "string" && value ? value : null;
}

function boundedNumber(value: unknown): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
}

// A display kind outside the closed set is a newer host talking to an older
// browser. It normalizes to "unstated" — both affordances — because a card
// matching on an unknown string would render NEITHER, which is a display step
// with nothing on it.
function knownDisplayKind(value: unknown): DeviceLinkDisplayKind | null {
  const kind = optionalText(value);
  return kind === DEVICE_LINK_DISPLAY_KINDS.link ||
    kind === DEVICE_LINK_DISPLAY_KINDS.qrCode
    ? kind
    : null;
}

// `input_kind`, `mode`, `restartable`, and `flow_id` are PROPOSAL §8.12's
// additive frame fields. The backend now emits all four (`DeviceLinkPromptView`
// carries them, projected from the durable flow record), so the fallbacks below
// are DEFENSIVE ONLY — they keep a card rendering against an older server
// rather than blanking:
//   - `input_kind` -> "code" (the masked-password affordance needs the real
//     field; falling back shows a visible input where a cloud password belongs),
//   - `mode` -> "default" (the "use the other path" switch needs the real one),
//   - `restartable` -> derived from `error_code`,
//   - `flow_id` -> absent, and a card with no flow id cannot poll or submit,
//     so it starts a flow of its own instead.
//
// The recipe-shaped fields are newer still, and each falls back to what a card
// did before it existed — never to one vendor's ceremony:
//   - `alternate_available` -> false. FAIL-CLOSED on purpose: a vendor that
//     declares no second path answers `UnsupportedMode`, so a card that assumed
//     one offers a switch that wedges the user,
//   - `default_mode_label` / `alternate_mode_label` -> absent, and the card
//     labels the switch from generic host copy,
//   - `display_kind` -> absent, and the card renders both affordances,
//   - `vendor_user_ref` -> absent, and a completed card shows no account line
//     rather than borrowing `code`, which means only "a short code the vendor
//     issued for the user to read",
//   - `extension_id` -> absent; `provider` is the credential authority, not the
//     installed extension, so neither substitutes for the other.
export function deviceLinkFrameFromWire(wire: unknown): DeviceLinkFrame | null {
  if (!wire || typeof wire !== "object") return null;
  const record = wire as Record<string, unknown>;
  const step = optionalText(record.step);
  if (!step) return null;
  const expiresAt = Date.parse(optionalText(record.expires_at) || "");
  const errorCode = optionalText(record.error_code);
  return {
    flowId: optionalText(record.flow_id),
    provider: String(record.provider || ""),
    extensionId: optionalText(record.extension_id),
    displayName: String(record.display_name || ""),
    step,
    instructions: String(record.instructions || ""),
    qrPayload: optionalText(record.qr_payload),
    displayKind: knownDisplayKind(record.display_kind),
    code: optionalText(record.code),
    vendorUserRef: optionalText(record.vendor_user_ref),
    secretLabel: optionalText(record.secret_label),
    inputKind: optionalText(record.input_kind) || DEVICE_LINK_INPUT_KINDS.code,
    mode: optionalText(record.mode) || DEVICE_LINK_MODES.default,
    alternateAvailable: record.alternate_available === true,
    defaultModeLabel: optionalText(record.default_mode_label),
    alternateModeLabel: optionalText(record.alternate_mode_label),
    expiresAtMs: Number.isFinite(expiresAt) ? expiresAt : 0,
    revision: boundedNumber(record.revision),
    pollIntervalMs:
      boundedNumber(record.poll_interval_ms) || DEVICE_LINK_DEFAULT_POLL_MS,
    // A vendor back-off overrides the pace for the next poll only, so it stays
    // a distinct field rather than being folded into `pollIntervalMs`.
    retryAfterMs: boundedNumber(record.retry_after_ms) || 0,
    errorCode,
    restartable:
      typeof record.restartable === "boolean"
        ? record.restartable
        : !errorCode || !NON_RESTARTABLE_ERROR_CODES.includes(errorCode),
    terminal: deviceLinkStepIsTerminal(step),
  };
}

// The delay before the next poll: a vendor-requested back-off wins, otherwise
// the frame's own pace.
export function deviceLinkPollDelayMs(
  frame:
    | Pick<DeviceLinkFrame, "pollIntervalMs" | "retryAfterMs">
    | null
    | undefined,
): number {
  if (!frame) return DEVICE_LINK_DEFAULT_POLL_MS;
  return frame.retryAfterMs || frame.pollIntervalMs || DEVICE_LINK_DEFAULT_POLL_MS;
}

// The mode a "use the other path instead" affordance switches to.
export function deviceLinkAlternateMode(mode: unknown): DeviceLinkMode {
  return mode === DEVICE_LINK_MODES.alternate
    ? DEVICE_LINK_MODES.default
    : DEVICE_LINK_MODES.alternate;
}

// The recipe's own name for one of the two paths, when it supplied one.
//
// The recipe promises that the words a user reads about linking come from the
// extension, so this is what a switch is labelled with; the null return is the
// signal to fall back to generic host copy. Which label goes with which mode is
// decided here, once, rather than at each surface that renders a switch.
export function deviceLinkModeLabel(
  frame: DeviceLinkFrame | null | undefined,
  mode: unknown,
): string | null {
  if (!frame) return null;
  return mode === DEVICE_LINK_MODES.alternate
    ? frame.alternateModeLabel || null
    : frame.defaultModeLabel || null;
}
