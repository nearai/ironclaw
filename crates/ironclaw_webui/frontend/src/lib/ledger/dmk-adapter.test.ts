import assert from "node:assert/strict";
import { describe, test } from "vitest";

import { DEVICE_STATUS } from "./device-port";
import {
  descriptorProxyUrl,
  mapDeviceActionError as mapDeviceActionErrorRaw,
  mapSessionState,
} from "./dmk-adapter";

/** The mapper returns Error subclasses that carry a discriminant flag. */
const mapDeviceActionError = mapDeviceActionErrorRaw as (
  error: unknown,
) => Error & { rejected?: boolean; disconnected?: boolean };

describe("session state mapping", () => {
  /**
   * This mapping picks which of five instructions the user is given. Telling
   * someone to unlock a device that is actually running the wrong app sends
   * them in circles.
   */
  test("each device condition maps to its own status", () => {
    assert.equal(mapSessionState({ deviceStatus: "LOCKED" }), DEVICE_STATUS.locked);
    assert.equal(
      mapSessionState({ deviceStatus: "NOT CONNECTED" }),
      DEVICE_STATUS.disconnected,
    );
    assert.equal(
      mapSessionState({ deviceStatus: "CONNECTED", currentApp: { name: "Ethereum" } }),
      DEVICE_STATUS.ready,
    );
    assert.equal(
      mapSessionState({ deviceStatus: "CONNECTED", currentApp: { name: "Bitcoin" } }),
      DEVICE_STATUS.wrongApp,
    );
  });

  /** A connected device sitting on the dashboard is not ready to sign. */
  test("connected with no app open is wrong-app, not ready", () => {
    assert.equal(
      mapSessionState({ deviceStatus: "CONNECTED" }),
      DEVICE_STATUS.wrongApp,
      "no app open must never read as ready",
    );
  });

  /** DMK reports the app name either bare or wrapped; both must work. */
  test("the app name is read in either shape", () => {
    assert.equal(
      mapSessionState({ deviceStatus: "CONNECTED", currentApp: "Ethereum" }),
      DEVICE_STATUS.ready,
    );
  });

  /** Absent state must fail closed, never optimistically ready. */
  test("an absent or unknown state is disconnected", () => {
    assert.equal(mapSessionState(undefined), DEVICE_STATUS.disconnected);
    assert.equal(mapSessionState(null), DEVICE_STATUS.disconnected);
    assert.equal(
      mapSessionState({ deviceStatus: "SOMETHING_NEW", currentApp: "Ethereum" }),
      DEVICE_STATUS.disconnected,
      "an unrecognized status must not be treated as ready",
    );
  });
});

describe("device error mapping", () => {
  /**
   * The Ethereum app returns 0x6985 when the human presses reject. That must
   * become a rejection and nothing else — reporting it as a generic failure
   * would suggest retrying something the user deliberately refused.
   */
  test("an on-device rejection is recognized by tag and by status word", () => {
    assert.equal(mapDeviceActionError({ _tag: "UserRejectedError" }).rejected, true);
    assert.equal(
      mapDeviceActionError({ message: "Ledger error 0x6985" }).rejected,
      true,
    );
    assert.equal(
      mapDeviceActionError({ message: "Transaction denied by the user" }).rejected,
      true,
    );
  });

  test("a disconnect is recognized by tag and by message", () => {
    assert.equal(
      mapDeviceActionError({ _tag: "DeviceDisconnectedError" }).disconnected,
      true,
    );
    assert.equal(
      mapDeviceActionError({ message: "No accessible device" }).disconnected,
      true,
    );
  });

  /**
   * An unrecognized failure must NOT be guessed at. Labelling it a rejection
   * would tell the user a falsehood about their own action.
   */
  test("an unknown failure is passed through, not guessed at", () => {
    const mapped = mapDeviceActionError(new Error("USB bus reset"));
    assert.equal(mapped.rejected, undefined);
    assert.equal(mapped.disconnected, undefined);
    assert.match(mapped.message, /USB bus reset/);
  });

  test("a non-Error value still yields an Error", () => {
    const mapped = mapDeviceActionError("something odd");
    assert.ok(mapped instanceof Error);
  });
});

describe("egress", () => {
  /** Descriptors must come from our same-origin proxy, never Ledger's CAL. */
  test("the descriptor URL is same-origin and intent-scoped", () => {
    const url = descriptorProxyUrl("01J000INTENT");
    assert.equal(url, "/api/webchat/v2/intents/01J000INTENT/signing-context");
    assert.ok(!url.includes("://"), "must not be an absolute remote URL");
    assert.ok(!url.includes("ledger.com"));
  });

  test("an intent id is encoded rather than interpolated raw", () => {
    assert.ok(descriptorProxyUrl("a/../b").includes("a%2F..%2Fb"));
  });
});
