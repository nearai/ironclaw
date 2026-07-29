import assert from "node:assert/strict";
import { describe, test } from "vitest";

import { CEREMONY_OUTCOME, runCeremony } from "./ceremony";
import {
  DEVICE_STATUS,
  DeviceDisconnectedError,
  DeviceRejectedError,
  createScriptedDevice,
} from "./device-port";

const INTENT = {
  intent_id: "01J0000000000000000000CEREMONY",
  unsigned_payload: "0x02f8710182...server-supplied",
  derivation_path: "44'/60'/0'/0/0",
};

const DESCRIPTOR = { context: { contract: {} }, display: { formats: {} } };

function run(overrides = {}) {
  return runCeremony({
    intent: INTENT,
    clearSigningAvailable: true,
    descriptor: DESCRIPTOR,
    ...overrides,
  });
}

describe("the §D5 device scenarios", () => {
  test("happy path: a signature comes back with its metadata", async () => {
    const device = createScriptedDevice({
      signResult: {
        signature: "0xsig",
        deviceMetadata: { model: "nanoSPlus", appVersion: "1.12.0" },
      },
    });

    const result = await run({ device });

    assert.equal(result.outcome, CEREMONY_OUTCOME.signed);
    assert.equal(result.signature, "0xsig");
    assert.equal(result.derivationPath, INTENT.derivation_path);
    assert.deepEqual(result.deviceMetadata, {
      model: "nanoSPlus",
      appVersion: "1.12.0",
    });
    assert.equal(device.calls.disconnect.length, 1, "the device is released");
  });

  test("device-rejected: the human declines on the device", async () => {
    const device = createScriptedDevice({ signError: new DeviceRejectedError() });
    const result = await run({ device });

    assert.equal(result.outcome, CEREMONY_OUTCOME.rejected);
    assert.equal(result.signature, undefined, "a refusal carries no signature");
  });

  test("disconnect mid-sign is its own outcome, not a generic failure", async () => {
    const device = createScriptedDevice({ signError: new DeviceDisconnectedError() });
    const result = await run({ device });

    assert.equal(result.outcome, CEREMONY_OUTCOME.disconnected);
  });

  test("wrong app: the ceremony stops before requesting a signature", async () => {
    const device = createScriptedDevice({ statusSequence: [DEVICE_STATUS.wrongApp] });
    const result = await run({ device });

    assert.equal(result.outcome, CEREMONY_OUTCOME.wrongApp);
    assert.equal(
      device.calls.signTransaction.length,
      0,
      "nothing may be sent to a device in the wrong app",
    );
  });

  test("locked: the ceremony stops before requesting a signature", async () => {
    const device = createScriptedDevice({ statusSequence: [DEVICE_STATUS.locked] });
    const result = await run({ device });

    assert.equal(result.outcome, CEREMONY_OUTCOME.locked);
    assert.equal(device.calls.signTransaction.length, 0);
  });
});

describe("the rules the SDK is not trusted with", () => {
  /**
   * THE fail-closed property. Without a descriptor the device cannot render
   * what it is signing, so it must never be asked — not asked-and-declined,
   * and certainly not asked to blind-sign.
   */
  test("no descriptor: the device is never contacted at all", async () => {
    const device = createScriptedDevice({ signResult: { signature: "0xsig" } });

    const result = await run({ device, clearSigningAvailable: false });

    assert.equal(result.outcome, CEREMONY_OUTCOME.blocked);
    assert.equal(device.calls.signTransaction.length, 0, "no signature requested");
    assert.equal(
      device.calls.connect.length,
      0,
      "the device is not even connected — a device never contacted cannot be talked into signing",
    );
  });

  /**
   * The browser must sign what the server bound, byte for byte. It has no code
   * that builds or re-encodes a transaction, and this pins that the payload
   * reaches the device unmodified.
   */
  test("the unsigned payload reaches the device byte-identically", async () => {
    const device = createScriptedDevice({ signResult: { signature: "0xsig" } });
    await run({ device });

    assert.equal(device.calls.signTransaction.length, 1);
    const request = device.calls.signTransaction[0];
    assert.equal(
      request.unsignedPayload,
      INTENT.unsigned_payload,
      "the payload must pass through untouched",
    );
    assert.equal(request.derivationPath, INTENT.derivation_path);
    assert.deepEqual(
      request.clearSigningContext,
      DESCRIPTOR,
      "the descriptor the device renders from must be the one the server served",
    );
  });

  /** A refusal is a value, so no caller can catch its way into a success path. */
  test("every refusal is a returned outcome, never a thrown error", async () => {
    const cases = [
      { device: createScriptedDevice({ signError: new DeviceRejectedError() }) },
      { device: createScriptedDevice({ statusSequence: [DEVICE_STATUS.locked] }) },
      { clearSigningAvailable: false, device: createScriptedDevice({}) },
      { device: createScriptedDevice({ signError: new Error("bus error") }) },
      { device: undefined },
    ];

    for (const overrides of cases) {
      const result = await run(overrides);
      assert.ok(result?.outcome, "an outcome is always returned");
      assert.notEqual(
        result.outcome,
        CEREMONY_OUTCOME.signed,
        "no refusal may be mistaken for a signature",
      );
    }
  });

  /** A malformed server response must not become a device prompt. */
  test("a missing unsigned payload is refused before the device is contacted", async () => {
    const device = createScriptedDevice({ signResult: { signature: "0xsig" } });
    const result = await run({ device, intent: { intent_id: "x" } });

    assert.equal(result.outcome, CEREMONY_OUTCOME.failed);
    assert.equal(device.calls.connect.length, 0);
  });

  /** A device that returns nothing useful is a failure, not a silent success. */
  test("a device that returns no signature fails rather than reporting success", async () => {
    const device = createScriptedDevice({ signResult: {} });
    const result = await run({ device });

    assert.equal(result.outcome, CEREMONY_OUTCOME.failed);
  });

  /** Releasing the device must never mask the outcome. */
  test("a failure to disconnect does not change the result", async () => {
    const device = createScriptedDevice({ signResult: { signature: "0xsig" } });
    device.disconnect = async () => {
      throw new Error("release failed");
    };

    const result = await run({ device });
    assert.equal(result.outcome, CEREMONY_OUTCOME.signed);
  });
});
