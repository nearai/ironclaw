// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { PAIRING_REDEEM_PATH, redeemPairingCode } from "./pairing-api";

function installFetch(handler) {
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;
  const restore = () => {
    globalThis.fetch = originalFetch;
    globalThis.sessionStorage = originalSessionStorage;
  };

  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = handler;
  return restore;
}

test("redeemPairingCode posts channel and code through the v2 extensions endpoint", async () => {
  const calls = [];
  const restore = installFetch(async (path, options) => {
    calls.push({ path, options });
    return new Response(
      JSON.stringify({ provider: "slack", provider_user_id: "install-alpha:U123" }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      }
    );
  });

  try {
    const response = await redeemPairingCode("slack", "A1B2C3");

    assert.deepEqual(response, {
      success: true,
      provider: "slack",
      provider_user_id: "install-alpha:U123",
      resumeError: false,
      resumedRunCount: 0,
    });
    assert.equal(calls.length, 1);
    assert.equal(calls[0].path, PAIRING_REDEEM_PATH);
    assert.equal(calls[0].options.method, "POST");
    assert.equal(calls[0].options.credentials, "same-origin");
    assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
    assert.equal(calls[0].options.headers.get("Content-Type"), "application/json");
    assert.deepEqual(JSON.parse(calls[0].options.body), {
      channel: "slack",
      code: "A1B2C3",
    });
  } finally {
    restore();
  }
});

test("redeemPairingCode preserves the ApiError envelope on 400", async () => {
  const restore = installFetch(async () =>
    new Response(JSON.stringify({ error: "invalid_or_expired_code" }), {
      status: 400,
      headers: { "content-type": "application/json" },
    })
  );

  try {
    await assert.rejects(
      () => redeemPairingCode("slack", "BADCODE"),
      (error) => {
        assert.equal(error.name, "ApiError");
        assert.equal(error.status, 400);
        assert.equal(error.payload?.error, "invalid_or_expired_code");
        return true;
      }
    );
  } finally {
    restore();
  }
});
