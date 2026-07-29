// @ts-nocheck
/**
 * The hardware-device port (attested-signing §D4).
 *
 * The ceremony is written against this interface, not against Ledger's DMK.
 * Two reasons, and the second is the one that matters:
 *
 * 1. Every §D5 scenario — happy path, device-rejected, disconnect mid-sign,
 *    wrong app, locked — is scriptable here, so `vitest` needs no Ledger
 *    packages and no hardware.
 * 2. It keeps the ceremony's *decisions* out of the vendor SDK. Whether signing
 *    is permitted, what may be requested, and what the server is told are
 *    IronClaw's rules; the SDK's job is to move bytes to a device. A ceremony
 *    written directly on DMK would spread those rules through vendor callbacks
 *    where they are hard to see and harder to test.
 *
 * The real `@ledgerhq/device-management-kit` adapter implements this interface
 * and nothing else. If DMK's shape cannot satisfy it, that is a finding about
 * the ceremony's requirements, not a reason to widen the port.
 */

/** Device states the ceremony must distinguish, each a different user fix. */
export const DEVICE_STATUS = {
  /** No device selected yet. */
  disconnected: "disconnected",
  /** Device present but locked — the user must enter their PIN. */
  locked: "locked",
  /** Device unlocked, but the Ethereum app is not open. */
  wrongApp: "wrong-app",
  /** Ready to sign. */
  ready: "ready",
};

/**
 * A scripted device, for tests.
 *
 * Captures **every argument** of every call. Mock hygiene is not pedantry
 * here: the one thing the browser must never do is ask the device to sign
 * something other than what the server sent, and a double that dropped
 * arguments would let exactly that regression pass unnoticed.
 */
export function createScriptedDevice(
  {
    statusSequence = [DEVICE_STATUS.ready],
    signResult,
    signError,
  }: {
    statusSequence?: string[];
    signResult?: unknown;
    signError?: unknown;
  } = {},
) {
  const calls = { connect: [], status: [], signTransaction: [], disconnect: [] };
  let statusIndex = 0;

  return {
    calls,
    async connect(options) {
      calls.connect.push({ options });
      return { connected: true };
    },
    async status() {
      calls.status.push({});
      const next = statusSequence[Math.min(statusIndex, statusSequence.length - 1)];
      statusIndex += 1;
      return next;
    },
    async signTransaction(request) {
      // Recorded whole and unmodified — assertions compare against what the
      // server supplied, so a ceremony that mutated it would be caught.
      calls.signTransaction.push(request);
      if (signError) throw signError;
      return signResult;
    },
    async disconnect() {
      calls.disconnect.push({});
    },
  };
}

/** The error a device raises when the human declines on-device. */
export class DeviceRejectedError extends Error {
  constructor(message = "rejected on device") {
    super(message);
    this.name = "DeviceRejectedError";
    this.rejected = true;
  }
}

/** The error a device raises when it goes away mid-flow. */
export class DeviceDisconnectedError extends Error {
  constructor(message = "device disconnected") {
    super(message);
    this.name = "DeviceDisconnectedError";
    this.disconnected = true;
  }
}
