// @ts-nocheck
/**
 * The signing ceremony (attested-signing §D4).
 *
 * Drives a device through one signature and reports where it got to. It holds
 * the rules the vendor SDK must not be trusted with:
 *
 * * **No descriptor, no request.** If clear signing is unavailable the ceremony
 *   never reaches the device at all — not a request the device declines, not a
 *   blind-sign fallback. The device is never asked.
 * * **Sign only what the server sent.** The unsigned payload is passed through
 *   byte-identically. The browser has no code path that constructs, edits, or
 *   re-encodes a transaction, so there is nothing here that could sign
 *   something other than what was bound.
 * * **Every non-ready device state is its own outcome**, because each has a
 *   different human fix: unlock the device, open the Ethereum app, plug it
 *   back in.
 *
 * The result is a plain value, not a thrown error, so a caller cannot
 * accidentally `catch` its way past a refusal into a success path.
 */

import {
  DEVICE_STATUS,
  DeviceDisconnectedError,
  DeviceRejectedError,
} from "./device-port";

/** Terminal outcomes of a ceremony run. */
export const CEREMONY_OUTCOME = {
  signed: "signed",
  /** Clear signing unavailable — the device was never asked. */
  blocked: "blocked",
  /** The human declined on the device. */
  rejected: "rejected",
  /** Device locked; the user must enter their PIN. */
  locked: "locked",
  /** Ethereum app not open. */
  wrongApp: "wrong-app",
  /** Device went away mid-flow. */
  disconnected: "disconnected",
  /** No device port can exist here (no WebHID, or no adapter wired). */
  unsupported: "unsupported",
  /** Anything else, surfaced rather than swallowed. */
  failed: "failed",
};

/**
 * Run one ceremony.
 *
 * `intent` is the server's payload, passed to the device untouched.
 * `clearSigningAvailable` is the backend's answer, not a local guess.
 */
export async function runCeremony({
  device,
  intent,
  clearSigningAvailable,
  descriptor,
} = {}) {
  // Checked FIRST, before connect: a device that is never contacted cannot be
  // talked into signing. This ordering is the fail-closed property — a later
  // check would still be correct, but only as long as nothing between here and
  // there ever asked the device for a signature.
  if (!clearSigningAvailable) {
    return { outcome: CEREMONY_OUTCOME.blocked };
  }

  if (!device) {
    // Distinct from `failed`: nothing went wrong, this browser simply cannot
    // reach a device. The user's fix is a different browser, not a retry.
    return { outcome: CEREMONY_OUTCOME.unsupported };
  }
  if (!intent || !intent.unsigned_payload) {
    // Nothing to sign means nothing to ask for. Refusing here keeps a
    // malformed server response from becoming a device prompt the human is
    // asked to trust.
    return { outcome: CEREMONY_OUTCOME.failed, reason: "no unsigned payload" };
  }

  try {
    await device.connect({ intentId: intent.intent_id });

    const status = await device.status();
    if (status === DEVICE_STATUS.locked) {
      return { outcome: CEREMONY_OUTCOME.locked };
    }
    if (status === DEVICE_STATUS.wrongApp) {
      return { outcome: CEREMONY_OUTCOME.wrongApp };
    }
    if (status !== DEVICE_STATUS.ready) {
      return { outcome: CEREMONY_OUTCOME.disconnected };
    }

    const result = await device.signTransaction({
      // Byte-identical passthrough of what the server bound.
      unsignedPayload: intent.unsigned_payload,
      derivationPath: intent.derivation_path,
      // The descriptor the device renders from. Present by construction:
      // we returned `blocked` above when clear signing was unavailable.
      clearSigningContext: descriptor,
    });

    if (!result || !result.signature) {
      return { outcome: CEREMONY_OUTCOME.failed, reason: "device returned no signature" };
    }
    return {
      outcome: CEREMONY_OUTCOME.signed,
      signature: result.signature,
      derivationPath: intent.derivation_path,
      deviceMetadata: result.deviceMetadata,
    };
  } catch (error) {
    if (error instanceof DeviceRejectedError || error?.rejected) {
      return { outcome: CEREMONY_OUTCOME.rejected };
    }
    if (error instanceof DeviceDisconnectedError || error?.disconnected) {
      return { outcome: CEREMONY_OUTCOME.disconnected };
    }
    return { outcome: CEREMONY_OUTCOME.failed, reason: error?.message };
  } finally {
    // Best-effort: a device that already vanished cannot be disconnected, and
    // failing to release must not mask the outcome we are returning.
    try {
      await device?.disconnect?.();
    } catch (_) {
      /* the outcome above is what matters */
    }
  }
}
