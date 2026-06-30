import assert from "node:assert/strict";
import test from "node:test";

import { SLACK_PAIRING_REDEEM_PATH, redeemSlackPairingCode } from "./slack-pairing-api.js";

test("redeemSlackPairingCode posts Slack codes to the Reborn pairing endpoint", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(
      JSON.stringify({ provider: "slack", provider_user_id: "install-alpha:U123" }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      }
    );
  };

  const response = await redeemSlackPairingCode("A1B2C3");

  assert.deepEqual(response, {
    success: true,
    provider: "slack",
    provider_user_id: "install-alpha:U123",
    message: "Slack account connected.",
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, SLACK_PAIRING_REDEEM_PATH);
  assert.equal(calls[0].options.method, "POST");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
  assert.equal(calls[0].options.headers.get("Content-Type"), "application/json");
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    channel: "slack",
    code: "A1B2C3",
  });
});
