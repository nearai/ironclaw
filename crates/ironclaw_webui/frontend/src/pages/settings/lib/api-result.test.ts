import assert from "node:assert/strict";
import { test } from "vitest";

import { throwIfApiFailed } from "./api-result";

test("throwIfApiFailed throws on success:false so the mutation rejects", () => {
  // The settings/tools save bug: a stub resolves { success: false } and the
  // mutation's onSuccess still fires a fake "Saved". This guard turns it into
  // a rejection instead.
  assert.throws(() => throwIfApiFailed({ success: false, message: "Conflict" }), /Conflict/);
});

test("throwIfApiFailed uses the fallback message when none is provided", () => {
  assert.throws(() => throwIfApiFailed({ success: false }, "Save failed"), /Save failed/);
});

test("throwIfApiFailed returns the data unchanged on success", () => {
  const ok = { success: true, value: 1 };
  assert.equal(throwIfApiFailed(ok), ok);
  // Responses without an explicit success flag are treated as success.
  const passthrough = { value: 2 };
  assert.equal(throwIfApiFailed(passthrough), passthrough);
  assert.equal(throwIfApiFailed(null), null);
});
