import assert from "node:assert/strict";
import { test } from "vitest";

import { pairingErrorMessage } from "./pairing-errors";

test("pairingErrorMessage uses the server envelope before generic error text", () => {
  assert.equal(pairingErrorMessage({ payload: { error: "a" }, message: "x" }, "fb"), "a");
  assert.equal(pairingErrorMessage({ payload: { message: "b" }, message: "x" }, "fb"), "b");
  assert.equal(pairingErrorMessage({ message: "c" }, "fb"), "c");
  assert.equal(pairingErrorMessage(undefined, "fb"), "fb");
});
